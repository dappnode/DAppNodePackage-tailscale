import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
BOOT = r'''
import json, os, pathlib, signal, sys, time
calls = pathlib.Path(os.environ['TEST_CALLS'])
previous = calls.read_text().splitlines() if calls.exists() else []
with calls.open('a') as f:
    f.write(json.dumps({'key': os.getenv('TS_AUTHKEY'), 'alias': os.getenv('TS_AUTH_KEY'),
                        'args': sys.argv[1:], 'pid': os.getpid(),
                        'stateDir': os.getenv('TS_STATE_DIR'),
                        'timeout': os.getenv('TS_BOOT_TIMEOUT')}) + '\n')
results = json.loads(os.environ['TEST_RESULTS'])
if len(previous) >= len(results):
    sys.exit(99)
result = results[len(previous)]
if result.get('wait'):
    def stop(*_):
        pathlib.Path(os.environ['TEST_STOPPED']).write_text('stopped')
        sys.exit(0)
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    print('READY', flush=True)
    while True:
        signal.pause()
print(result.get('log', ''), flush=True)
sys.exit(result.get('exit', 0))
'''
STATUS = r'''
import os, sys
assert sys.argv[1:] == ['status', '--json']
print(os.environ.get('TEST_STATUS', ''))
sys.exit(int(os.environ.get('TEST_STATUS_EXIT', '0')))
'''


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        self.bin = self.dir / 'bin'
        self.bin.mkdir()
        for name, content in [('containerboot', BOOT), ('tailscale', STATUS)]:
            path = self.bin / name
            path.write_text('#!' + sys.executable + '\n' + content)
            path.chmod(0o755)
        self.state = self.dir / 'state'
        self.state.mkdir()
        (self.state / 'tailscaled.state').write_text('synthetic saved identity')
        self.scratch = self.dir / 'scratch'
        self.scratch.mkdir()
        self.env = {k: v for k, v in os.environ.items()
                    if not k.startswith(('TS_', 'TEST_'))}
        self.env.update({
            'PATH': str(self.bin) + os.pathsep + os.environ['PATH'],
            'TMPDIR': str(self.scratch),
            'TEST_CALLS': str(self.dir / 'calls'),
            'TEST_STOPPED': str(self.dir / 'stopped'),
            'TS_STATE_DIR': str(self.state),
            'TS_BOOT_TIMEOUT': '1h',
        })

    def calls(self):
        return [json.loads(line) for line in (self.dir / 'calls').read_text().splitlines()]

    def boot(self, results, **extra):
        env = {**self.env, 'TEST_RESULTS': json.dumps(results), **extra}
        return subprocess.run(['sh', str(ROOT / 'scripts/entrypoint.sh'), '--test-arg'],
                              env=env, capture_output=True, text=True, timeout=5)

    def test_browser_login_without_key_uses_upstream_directly(self):
        result = self.boot([{'log': 'browser login'}])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 1)
        self.assertIsNone(self.calls()[0]['key'])

    def test_valid_or_saved_login_does_not_retry(self):
        result = self.boot([{'log': 'saved login reused'}], TS_AUTHKEY='synthetic-key')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 1)
        self.assertEqual(self.calls()[0]['key'], 'synthetic-key')

    def test_rejected_keys_fall_back_once_without_either_alias(self):
        result = self.boot([
            {'exit': 1, 'log': 'backend error: invalid key: API key does not exist'},
            {'log': 'browser login URL'},
        ], TS_AUTHKEY='synthetic-key', TS_AUTH_KEY='synthetic-alias')
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual(len(calls), 2)
        self.assertIsNone(calls[1]['key'])
        self.assertIsNone(calls[1]['alias'])
        self.assertEqual(calls[1]['args'], ['--test-arg'])
        self.assertEqual(calls[1]['timeout'], '1h')
        self.assertIn('Falling back to browser login', result.stdout)
        self.assertNotIn('synthetic-key', result.stdout + result.stderr)
        self.assertEqual((self.state / 'tailscaled.state').read_text(), 'synthetic saved identity')
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_expired_key_using_upstream_alias_recovers(self):
        result = self.boot([
            {'exit': 1, 'log': '2026/10/23 Received error: invalid key: key has expired'},
            {'log': 'waiting for browser login'},
        ], TS_AUTH_KEY='synthetic-expired-key')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 2)
        self.assertIsNone(self.calls()[1]['alias'])

    def test_consumed_key_recovers(self):
        result = self.boot([
            {'exit': 1, 'log': 'Error: invalid key: key already used'},
            {'exit': 0},
        ], TS_AUTHKEY='synthetic-consumed-key')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 2)

    def test_network_and_tls_errors_do_not_discard_key_or_retry(self):
        for log in ['control: connection timed out', 'x509: certificate signed by unknown authority']:
            with self.subTest(log=log):
                (self.dir / 'calls').unlink(missing_ok=True)
                result = self.boot([{'exit': 23, 'log': log}], TS_AUTHKEY='synthetic-key')
                self.assertEqual(result.returncode, 23, result.stderr)
                self.assertEqual(len(self.calls()), 1)
                self.assertNotIn('Falling back', result.stdout)

    def test_fallback_failure_keeps_exit_status_without_looping(self):
        result = self.boot([
            {'exit': 1, 'log': 'backend error: invalid key: key has expired'},
            {'exit': 17, 'log': 'startup timed out'},
        ], TS_AUTHKEY='synthetic-key')
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(len(self.calls()), 2)

    def test_success_does_not_retry_even_if_old_error_is_logged(self):
        result = self.boot([{'log': 'backend error: invalid key: historical warning'}],
                           TS_AUTHKEY='synthetic-key')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 1)

    def test_term_reaches_daemon_and_does_not_start_fallback(self):
        env = {**self.env, 'TEST_RESULTS': json.dumps([{'wait': True}]),
               'TS_AUTHKEY': 'synthetic-key'}
        proc = subprocess.Popen(['sh', str(ROOT / 'scripts/entrypoint.sh')],
                                env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                text=True, start_new_session=True)
        try:
            self.assertTrue(select.select([proc.stdout], [], [], 5)[0], 'daemon did not start')
            self.assertEqual(proc.stdout.readline().strip(), 'READY')
            proc.send_signal(signal.SIGTERM)
            output, _ = proc.communicate(timeout=5)
            self.assertEqual(proc.returncode, 143, output)
            self.assertTrue((self.dir / 'stopped').exists())
            self.assertEqual(len(self.calls()), 1)
            self.assertEqual(list(self.scratch.iterdir()), [])
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.communicate()

    def health(self, status, exit_code=0):
        return subprocess.run(['sh', str(ROOT / 'scripts/healthcheck.sh')],
                              env={**self.env, 'TEST_STATUS': json.dumps(status),
                                   'TEST_STATUS_EXIT': str(exit_code)},
                              capture_output=True, text=True, timeout=5)

    def test_health_accepts_online_running_with_no_warnings(self):
        for health in [None, []]:
            result = self.health({'BackendState': 'Running', 'Self': {'Online': True}, 'Health': health})
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_health_rejects_login_wait_offline_and_router_errors(self):
        for status in [
            {'BackendState': 'NeedsLogin'},
            {'BackendState': 'Running', 'Self': {'Online': False}},
            {'BackendState': 'Running', 'Self': {'Online': True}, 'Health': ['firewall setup failed']},
        ]:
            with self.subTest(status=status):
                result = self.health(status)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('Tailscale state=', result.stderr)
                if status.get('Health'):
                    self.assertIn('firewall setup failed', result.stderr)

    def test_health_rejects_unavailable_daemon(self):
        result = self.health({}, exit_code=1)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('unavailable', result.stderr)

    def test_health_rejects_malformed_status(self):
        result = subprocess.run(['sh', str(ROOT / 'scripts/healthcheck.sh')],
                                env={**self.env, 'TEST_STATUS': 'not json'},
                                capture_output=True, text=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)


if __name__ == '__main__':
    unittest.main()

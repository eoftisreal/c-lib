"""Harness regressions; synthetic commands are not AV1 conformance evidence."""
import hashlib
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import conformance


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        source = self.root / 'input.obu'
        source.write_bytes(b'fixture')
        self.case = dict(id='sample', path='input.obu', sha256=conformance.digest(source),
                         features=['8-bit'], width=2, height=2, bit_depth=8, layout='420', frames=1)

    def test_absent_decoder_and_missing_fixture_are_blocked(self):
        report = conformance.run([self.case], self.root)
        self.assertEqual(report['features']['8-bit'], 'BLOCKED')
        self.case['path'] = None
        self.assertIn('fixture not installed', conformance.run([self.case], self.root)['cases'][0]['reason'])

    def test_hash_mismatch_is_failure_even_without_decoder(self):
        (self.root / 'input.obu').write_bytes(b'changed')
        self.assertEqual(conformance.run([self.case], self.root)['cases'][0]['status'], 'FAIL')

    def simulate(self, count=6, comparison=0, decode_status=0):
        calls = []
        def execute(argv, values, timeout):
            calls.append((argv, values))
            if 'output' in values:
                Path(values['output']).write_bytes(b'\0' * count)
                code = decode_status
            else:
                code = comparison
            return dict(argv=argv, returncode=code, stdout='', stderr='frame=1 plane=V x=0 y=0')
        with patch.object(conformance, 'execute', execute):
            report = conformance.run([self.case], self.root, ['candidate'], ['reference'], Path('compare'))
        return report['cases'][0], calls

    def test_exact_count_required_before_comparison(self):
        for count in (0, 5, 7, 12):
            row, calls = self.simulate(count=count)
            self.assertEqual(row['status'], 'FAIL')
            self.assertEqual(len(calls), 2)

    def test_mismatch_diagnostic_and_success(self):
        row, _ = self.simulate(comparison=1)
        self.assertEqual(row['status'], 'FAIL')
        self.assertIn('frame=1 plane=V', row['reason'])
        row, _ = self.simulate()
        self.assertEqual(row['status'], 'PASS')
        self.assertEqual(row['output_sha256']['candidate.yuv'], hashlib.sha256(bytes(6)).hexdigest())

    def test_failure_cannot_pass_with_output(self):
        row, calls = self.simulate(decode_status=1)
        self.assertEqual(row['status'], 'FAIL')
        self.assertEqual(len(calls), 1)

    def test_native_rgb_cannot_be_converted_to_pass(self):
        self.case['layout'] = 'gbr'
        row, calls = self.simulate()
        self.assertEqual(row['status'], 'BLOCKED')
        self.assertFalse(calls)

    def test_command_failure_and_timeout_are_failures(self):
        for argv in ([str(self.root / 'no-executable')],
                     [sys.executable, '-c', 'import time; time.sleep(10)']):
            row = conformance.run([self.case], self.root, argv, ['reference'], Path('compare'), .02)['cases'][0]
            self.assertEqual(row['status'], 'FAIL')

    def test_cli_exit_codes_and_recorded_blocked_snapshot(self):
        import subprocess
        import json
        manifest = self.root / 'manifest.json'
        report = self.root / 'report.json'
        command = [sys.executable, str(Path(conformance.__file__)), '--manifest', str(manifest),
                   '--root', str(self.root), '--report', str(report)]
        for cases in ([], [self.case]):
            manifest.write_text(json.dumps({'cases': cases}))
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 2)
        self.case['sha256'] = 'incorrect'
        manifest.write_text(json.dumps({'cases': [self.case]}))
        self.assertEqual(subprocess.run(command, capture_output=True).returncode, 1)
        self.assertEqual(json.loads(report.read_text())['cases'][0]['status'], 'FAIL')

    def test_process_output_is_bounded_and_shell_is_not_used(self):
        result = conformance.execute([sys.executable, '-c', 'print("x" * 10000)'], {}, 5)
        self.assertEqual(result['returncode'], 0)
        self.assertLessEqual(len(result['stdout']), 4096)
        result = conformance.execute([sys.executable, '-c', 'import sys; print(sys.argv[1])',
                                      '$(do-not-execute){literal}'], {}, 5)
        self.assertEqual(result['stdout'].strip(), '$(do-not-execute){literal}')


if __name__ == '__main__':
    unittest.main()

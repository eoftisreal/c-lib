"""Negative gates and benchmark contracts; mock decodes are not conformance evidence."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import acceptance_gate
import api_contract
import benchmark
import conformance
import decoder_fuzz
import feature_status
import release


class ProductionTests(unittest.TestCase):
    def test_checked_in_acceptance_fails_strict_gate(self):
        root = Path(__file__).resolve().parent
        report = json.loads((root / 'conformance-report.json').read_text())
        manifest = json.loads((root / 'conformance.json').read_text())
        self.assertTrue(acceptance_gate.failures(report, manifest, conformance.digest(root / 'conformance.json'), 'HEAD'))

    def test_unrun_cases_report_blockers_without_inventing_stage_failures(self):
        root = Path(__file__).resolve().parent
        report = json.loads((root / 'conformance-report.json').read_text())
        manifest = json.loads((root / 'conformance.json').read_text())
        errors = acceptance_gate.failures(report, manifest, conformance.digest(root / 'conformance.json'), 'HEAD')
        text = '\n'.join(errors)
        self.assertIn('BLOCKED: broad acceptance candidate command not supplied; constrained intra pixel gates are separate', text)
        self.assertIn('comparisons were not run', text)
        for invented in ('did not succeed', 'no matching native pixel digests', 'missing or changed fixture hash', 'executable hashes required'):
            self.assertNotIn(invented, text)
        self.assertEqual(sum(': BLOCKED:' in line for line in errors), len(manifest['cases']))

    def evidence(self):
        case = dict(id='one', features=['intra'], sha256='input')
        row = dict(id='one', status='PASS', input_sha256='input', output_sha256={'candidate.yuv': 'pixels', 'reference.yuv': 'pixels'},
                   candidate={'returncode': 0}, reference={'returncode': 0}, comparison={'returncode': 0})
        report = dict(cases=[row], manifest_sha256='manifest', commands={'candidate': ['a'], 'reference': ['b']},
                      revisions={'candidate': 'HEAD', 'reference': 'dav1d-build'},
                      executable_sha256={k: 'hash' for k in ('candidate', 'reference', 'comparator')}, features={'intra': 'PASS'})
        return report, dict(cases=[case])

    def test_gate_rejects_missing_duplicate_stale_and_failed_evidence(self):
        report, manifest = self.evidence()
        self.assertFalse(acceptance_gate.failures(report, manifest, 'manifest', 'HEAD'))
        for mutate in [lambda r: r.update(cases=[]), lambda r: r['cases'].append(r['cases'][0]),
                       lambda r: r['cases'][0].update(status='BLOCKED'), lambda r: r.update(manifest_sha256='stale'),
                       lambda r: r['revisions'].update(candidate='old'), lambda r: r.update(executable_sha256={}),
                       lambda r: r['cases'][0]['comparison'].update(returncode=1),
                       lambda r: r['cases'][0]['output_sha256'].update({'candidate.yuv': 'wrong'}),
                       lambda r: r['features'].update(intra='BLOCKED')]:
            changed = copy.deepcopy(report)
            mutate(changed)
            self.assertTrue(acceptance_gate.failures(changed, manifest, 'manifest', 'HEAD'))

    def test_abi_extracts_entire_layout_and_versions_changes(self):
        a = api_contract.declarations('typedef struct A { uint64_t x; uint32_t y; } A;\nvoid codec_a(void);')
        b = api_contract.declarations('typedef struct A { uint64_t x; uint64_t y; } A;\nvoid codec_a(void);')
        self.assertNotEqual(a, b)
        old = dict(major=1, minor=0, declarations=a)
        self.assertTrue(api_contract.compatibility(old, dict(old, declarations=b)))
        self.assertFalse(api_contract.compatibility(old, dict(major=2, minor=0, declarations=b)))
        self.assertTrue(api_contract.compatibility(old, dict(old, declarations=a+['void codec_b(void);'])))
        self.assertFalse(api_contract.compatibility(old, dict(major=1, minor=1, declarations=a+['void codec_b(void);'])))

    def test_benchmark_regressions_and_invalid_baselines(self):
        baseline = dict(context={'runner': 'fixed'}, cases=[dict(id='720p', status='PASS', median_seconds=1, samples=[{'seconds': 1}]*7)])
        current = copy.deepcopy(baseline)
        current['cases'][0]['median_seconds'] = 1.11
        self.assertTrue(benchmark.regressions(current, baseline, 10))
        current['cases'][0]['median_seconds'] = 1.05
        current['cases'][0]['samples'] = [{'seconds': 1.05}]*7
        self.assertFalse(benchmark.regressions(current, baseline, 10))
        for value in (0, -1, float('nan'), float('inf')):
            current['cases'][0]['median_seconds'] = value
            self.assertTrue(benchmark.regressions(current, baseline, 10))
        for bad in (dict(context={}, cases=[]), dict(context=baseline['context'], cases=[dict(id='720p', status='BLOCKED')])):
            self.assertTrue(benchmark.regressions(baseline, bad, 10))
        current = copy.deepcopy(baseline)
        current['cases'][0]['samples'] = [{}]
        self.assertTrue(benchmark.regressions(current, baseline, 10))

    def test_timed_runs_must_reproduce_verified_pixels(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'in.obu'
            source.write_bytes(b'compressed')
            case = dict(id='test', path='in.obu', sha256=conformance.digest(source), frames=1)
            def execute(argv, values, timeout):
                Path(values['output']).write_bytes(b'wrong')
                return {'returncode': 0}
            verified = dict(cases=[dict(status='PASS', output_sha256={'candidate.yuv': 'verified-hash'})])
            with patch.object(conformance, 'run', return_value=verified), patch.object(conformance, 'execute', execute):
                with self.assertRaisesRegex(ValueError, 'verified native pixels'):
                    benchmark.benchmark_case(case, root, dict(candidate=['decoder']), Path('compare'), 1, 5, 1)

    def test_primitives_cannot_satisfy_full_decoder_fuzzing(self):
        manifest = {'bin': [{'name': name, 'path': name + '.rs'} for name in decoder_fuzz.PRIMITIVE_TARGETS]}
        for name in decoder_fuzz.PRIMITIVE_TARGETS | {None, 'missing'}:
            self.assertIsNone(decoder_fuzz.target_source(name, manifest))

    def test_feature_report_keeps_component_and_picture_separate(self):
        catalog = dict(profiles=[dict(id=0, name='Main', decoded_picture_support=False)],
                       features=[dict(id='cdef', implementation='scalar-kernel-verified', acceptance_features=['cdef'], evidence='test.rs')])
        text = feature_status.render(catalog, {'features': {}})
        self.assertIn('scalar-kernel-verified | BLOCKED', text)
        self.assertIn('Unsupported', text)

    def test_archive_normalizes_order_metadata_and_gzip_time(self):
        with tempfile.TemporaryDirectory() as directory:
            a, b = [Path(directory) / name for name in ('a.tar.gz', 'b.tar.gz')]
            release.archive({'bin/a': b'x', 'docs/a': b'y'}, a, 123)
            release.archive({'docs/a': b'y', 'bin/a': b'x'}, b, 123)
            self.assertEqual(a.read_bytes(), b.read_bytes())
            import tarfile
            with tarfile.open(a) as tar:
                for item in tar.getmembers():
                    self.assertEqual((item.uid, item.gid, item.mtime), (0, 0, 123))

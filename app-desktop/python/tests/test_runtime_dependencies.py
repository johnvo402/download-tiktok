import json
from pathlib import Path
import unittest
try:
    from worker.runtime_dependencies import unused_packages
except ImportError as error:
    raise unittest.SkipTest('Install packaging for dependency graph tests') from error


class DependencyGraphTests(unittest.TestCase):
    def test_shared_transitive_packages_survive_uninstall(self):
        graph = {'whisperx': ['torch', 'transformers'], 'bandit-infer': ['torch', 'soundfile'],
                 'torch': ['numpy'], 'numpy': [], 'transformers': ['tokenizers'], 'tokenizers': [], 'soundfile': [], 'packaging': []}
        self.assertEqual(unused_packages(['bandit-infer'], graph), ['tokenizers', 'transformers', 'whisperx'])

    def test_cuda_and_headless_substitutions_keep_actual_provider(self):
        graph = {'rapidocr': ['onnxruntime>=1.7', 'opencv-python'], 'onnxruntime-gpu': [], 'opencv-python-headless': [], 'packaging': []}
        self.assertEqual(unused_packages(['rapidocr'], graph, cuda_ort=True), [])

    def test_extras_and_platform_markers(self):
        graph = {'root': ['other[gpu]', 'linux; sys_platform == "never"'], 'other': ['accel; extra == "gpu"'],
                 'accel': [], 'linux': [], 'packaging': []}
        self.assertEqual(unused_packages(['root'], graph), ['linux'])

    def test_cycles_and_normalized_distribution_names(self):
        graph = {'Foo_Bar': ['dep'], 'dep': ['foo-bar'], 'unused': [], 'packaging': []}
        self.assertEqual(unused_packages(['foo-bar'], graph), ['unused'])

    def test_matrix_all_features_reference_pinned_existing_requirements(self):
        root = Path(__file__).resolve().parents[1]
        matrix = json.loads((root / 'runtime-matrix.json').read_text(encoding='utf-8'))
        self.assertEqual(matrix['protocolVersion'], 2)
        for feature in matrix['features'].values():
            self.assertTrue((root / feature['requirements']).is_file())
            for component in feature['components']:
                self.assertIn(component, matrix['components'])


if __name__ == '__main__':
    unittest.main()

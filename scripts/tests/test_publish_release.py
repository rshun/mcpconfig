import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from publish_release import checksums, digest, expected_names, prepare, verify_asset


class ReleaseTests(unittest.TestCase):
    def candidate(self, legacy=False):
        files = {name: name.encode() for name in expected_names('0.1.0')}
        if legacy:
            files = {name.replace('MCP.Hub_', 'MCP Hub_'): data for name, data in files.items()}
        return {**files, 'SHA256SUMS': checksums(files)}

    def release(self, files):
        return {'draft': True, 'assets': [
            {'name': name, 'size': len(data), 'digest': digest(data), 'state': 'uploaded'}
            for name, data in files.items()
        ]}

    def test_new_release_has_matching_portable_manifest(self):
        files, backup = prepare(self.candidate(), '0.1.0', None)
        self.assertFalse(backup)
        self.assertEqual(len(files), 7)
        self.assertEqual(files['SHA256SUMS'], checksums({n: d for n, d in files.items() if n != 'SHA256SUMS'}))

    def test_failed_legacy_draft_backs_up_original_manifest(self):
        candidate = self.candidate(legacy=True)
        uploaded = {n.replace('MCP Hub_', 'MCP.Hub_'): d for n, d in candidate.items()}
        files, backup = prepare(candidate, '0.1.0', self.release(uploaded))
        self.assertTrue(backup)
        self.assertEqual(files['SHA256SUMS.source'], candidate['SHA256SUMS'])
        self.assertEqual(len(files), 8)
        self.assertEqual(files['SHA256SUMS'], checksums({n: d for n, d in files.items() if n != 'SHA256SUMS'}))

    def test_resume_after_backup_and_partial_upload(self):
        candidate = self.candidate(legacy=True)
        files, _ = prepare(candidate, '0.1.0', self.release({
            n.replace('MCP Hub_', 'MCP.Hub_'): d for n, d in candidate.items()
        }))
        partial = {n: d for n, d in files.items() if n != 'SHA256SUMS'}
        resumed, backup = prepare(candidate, '0.1.0', self.release(partial))
        self.assertFalse(backup)
        self.assertEqual(resumed, files)
        complete, backup = prepare(candidate, '0.1.0', self.release(files))
        self.assertEqual(complete, files)
        self.assertFalse(backup)

    def test_corruption_and_unexpected_assets_fail_before_mutation(self):
        candidate = self.candidate()
        corrupt = dict(candidate, **{'customer-mcp.yaml': b'corrupt'})
        with self.assertRaises(ValueError):
            prepare(corrupt, '0.1.0', None)
        with self.assertRaises(ValueError):
            prepare(candidate, '0.1.0', self.release({'unexpected.txt': b'extra'}))
        broken = self.release(candidate)
        broken['assets'][0]['digest'] = digest(b'corrupt')
        with self.assertRaises(ValueError):
            prepare(candidate, '0.1.0', broken)

    def test_published_release_is_immutable(self):
        release = self.release(self.candidate())
        release['draft'] = False
        with self.assertRaises(ValueError):
            prepare(self.candidate(), '0.1.0', release)

    def test_missing_digest_or_incomplete_upload_is_rejected(self):
        files = {'asset': b'content'}
        asset = self.release(files)['assets'][0]
        asset['digest'] = None
        with self.assertRaises(ValueError):
            verify_asset(asset, files)
        asset['digest'] = digest(files['asset'])
        asset['state'] = 'starter'
        with self.assertRaises(ValueError):
            verify_asset(asset, files)


if __name__ == '__main__':
    unittest.main()

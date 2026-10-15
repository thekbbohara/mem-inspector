import importlib.util
import os
import pathlib
import subprocess
import tempfile
import unittest
spec = importlib.util.spec_from_file_location('storage', pathlib.Path(__file__).parents[1] / 'scripts/storage-overview.py')
storage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(storage)

class StorageTests(unittest.TestCase):
    def test_allocated_sizes_hardlinks_symlinks_and_nonoverlap(self):
        with tempfile.TemporaryDirectory() as root, tempfile.TemporaryDirectory() as outside:
            home = pathlib.Path(root)
            cache = home / '.cache' / 'nested'; cache.mkdir(parents=True)
            file = cache / 'payload'; file.write_bytes(b'x' * 2 * 1024 * 1024)
            os.link(file, home / 'hardlink')
            (cache / 'unshared').write_bytes(b'x' * 2 * 1024 * 1024)
            external = pathlib.Path(outside) / 'personal'; external.write_bytes(b'x' * 4 * 1024 * 1024)
            os.symlink(outside, home / 'external')
            (home / 'Documents').mkdir()
            result = storage.overview(root)
            expected = subprocess.check_output(['du','-sx','-B1','--',root]).split(b'\t')[0].decode()
            self.assertEqual(result['allocated'], expected)
            self.assertTrue(result['complete'])
            self.assertEqual(len(result['candidates']), 1)
            self.assertEqual(result['candidates'][0]['path'], str(home / '.cache'))
            independent = subprocess.check_output(['du', '-sx', '-B1', '--', str(home / '.cache')]).split(b'\t')[0].decode()
            self.assertEqual(result['candidates'][0]['scopeAllocated'], independent)
            self.assertFalse(any(r['path'].startswith(str(home / 'external')) for r in result['folders']))
            self.assertEqual(external.stat().st_size, 4 * 1024 * 1024)
    def test_personal_state_never_reclaim_candidate(self):
        for name in ['Documents/report', '.codex/history', '.jarvis/worktrees/project', '.local/share/models']:
            self.assertEqual(storage.classify('/home/test/' + name, '/home/test')[1], 'Keep')
    def test_kinds_repos_big_files_and_downloads(self):
        with tempfile.TemporaryDirectory() as root:
            home = pathlib.Path(root)
            def blob(rel, n):
                p = home / rel; p.parent.mkdir(parents=True, exist_ok=True); p.write_bytes(os.urandom(n)); return p
            mib = 1 << 20
            blob('Videos/trip.mkv', 3 * mib)
            blob('Pictures/a.jpg', mib)
            blob('code/app/src/main.rs', mib)
            blob('code/app/notes.unknownext', mib)
            (home / 'code/app/.git').mkdir(); blob('code/app/.git/HEAD', 10)
            blob('code/app/node_modules/x/index.js', 2 * mib)
            blob('.cache/thing/blob', mib)
            blob('models/llama.gguf', mib)
            blob('isos/arch.iso', mib)
            blob('misc/data.unknownext', mib)
            old = blob('Downloads/setup.run', mib)
            os.utime(old, (1_000_000_000, 1_000_000_000))
            storage.BIG_FILE = 2 * mib
            result = storage.overview(root)
            k = {key: int(v) for key, v in result['kinds'].items()}
            self.assertGreaterEqual(k['video'], 3 * mib)
            self.assertGreaterEqual(k['images'], mib)
            self.assertGreaterEqual(k['code'], 2 * mib, 'repo files, known or not, are code')
            self.assertGreaterEqual(k['deps'], 2 * mib)
            self.assertGreaterEqual(k['caches'], mib)
            self.assertGreaterEqual(k['models'], mib)
            self.assertGreaterEqual(k['archives'], mib)
            self.assertGreaterEqual(k['other'], 2 * mib)
            self.assertLessEqual(sum(k.values()), int(result['allocated']))
            self.assertEqual([r['path'] for r in result['repos']], [] , 'repos below 64 MiB are not listed')
            self.assertEqual({b['path'] for b in result['bigFiles']}, {str(home / 'Videos/trip.mkv'), str(home / 'code/app/node_modules/x/index.js')})
            self.assertEqual([(d['path'], d['newest']) for d in result['downloads']], [(str(old), 1_000_000_000)])
            only = storage.disk_kinds(root)
            self.assertEqual(only['kinds'], result['kinds'])
    def test_symlink_root_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            target=pathlib.Path(root) / 'link';target.symlink_to(root)
            with self.assertRaises(ValueError): storage.overview(str(target))

if __name__ == '__main__': unittest.main()

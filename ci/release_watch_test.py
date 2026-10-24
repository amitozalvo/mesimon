"""Offline release matching, pagination and checkpoint bookkeeping."""
import contextlib
from datetime import date
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("release_watch", Path(__file__).with_name("release-watch.py"))
watch = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(watch)


def fixture():
    return json.loads((watch.FIXTURES / "input.json").read_text())


class MatchingTests(unittest.TestCase):
    def test_dependencies_are_matched_case_insensitively_without_model_noise(self):
        for line in ("HOOKS changed", "plugin monitors", "a mod's Image", "mods API", "modules API",
                     "$.prompt.submit", "--plugin-dir", "MCP", "turn/steer", "app-server",
                     "queued input", "compaction events", "SessionStart", "UserPromptSubmit",
                     "PermissionRequest", "turn.step", "classic.PreToolUse", "clientUserMessageId",
                     "tool.check", "turn.complete", "tool.call", "StopFailure", "pasted_content",
                     "session.start", "agent.spawn", "SubagentStop", "paste", "interrupt"):
            with self.subTest(line=line):
                self.assertTrue(watch.watched(line))
        for line in ("Added a model", "modern logo colors", "commodity pricing", "Fixed typography"):
            self.assertFalse(watch.watched(line), line)

    def test_other_keeps_every_nonmatching_line_in_order(self):
        self.assertEqual(watch.partition(["- hooks", "  continuation", "", "- colors", "- settings"]),
                         (["- hooks", "- settings"], ["  continuation", "- colors"]))

    def test_upstream_markdown_cannot_close_a_diff_or_declaration_fence(self):
        self.assertEqual(watch.fenced(' ```\n+new', 'diff'), '````diff\n ```\n+new\n````')
        output = watch.grouped('```ts\n// hooks\n```', literal=True)
        self.assertIn('````text\n```ts\n```\n````', output)

    def test_versions_sort_numerically_and_preview_precedes_stable(self):
        self.assertLess(watch.version_key("2.1.9"), watch.version_key("2.1.10"))
        self.assertLess(watch.version_key("0.164.0-alpha.9"), watch.version_key("0.164.0-alpha.10"))
        self.assertLess(watch.version_key("0.164.0-alpha.10"), watch.version_key("0.164.0"))
        for text in ("v2.1.310", "2.1.310 (Claude Code)"):
            self.assertEqual(watch.version(text), "2.1.310")
        self.assertEqual(watch.version("codex-cli 0.164.0"), "0.164.0")
        for text in ("garbage", "v0.164", "addon-v0.164.0"):
            with self.assertRaises(ValueError):
                watch.version(text)


class ReleaseTests(unittest.TestCase):
    def test_all_intervening_releases_and_other_are_printed(self):
        f = fixture()
        current = watch.collect(f['state'], f['responses'].__getitem__, f['versions'], f['protocol'])
        output = watch.digest(f['state'], f['before'], current, dry=True)
        self.assertIn('synthetic offline fixtures', output)
        self.assertIn('### 2.1.10', output)
        self.assertIn('### 2.1.11', output)
        self.assertNotIn('### 2.1.12', output)
        self.assertNotIn('0.165.0-alpha', output)
        self.assertIn('#### Other\n\n- Changed logo colors.', output)
        self.assertIn('+export type TurnSteerParams', output)
        self.assertIn('clientUserMessageId', output)

    def test_paginates_past_previews_and_excludes_drafts(self):
        def release(v, **extra):
            return dict(tag_name='rust-v'+v, **extra)
        pages = [[release('0.166.0-alpha.1', prerelease=True), release('0.165.0', draft=True)],
                 [release('0.165.0'), release('0.164.0')], [release('0.163.0')]]
        seen = []
        def get(url):
            seen.append(url)
            return json.dumps(pages[len(seen)-1])
        rows = watch.releases('codex', '0.163.0', '0.165.0', get)
        self.assertEqual([v for v, _ in rows], ['0.164.0', '0.165.0'])
        self.assertEqual(len(seen), 3)
        self.assertTrue(seen[-1].endswith('page=3'))

    def test_prerelease_install_includes_intervening_prereleases(self):
        rows = [{'tag_name': 'rust-v'+v, 'prerelease': '-' in v} for v in
                ['0.164.0-alpha.10', '0.164.0-alpha.2', '0.163.0']]
        result = watch.releases('codex', '0.163.0', '0.164.0-alpha.10', lambda _: json.dumps(rows))
        self.assertEqual([v for v, _ in result], ['0.164.0-alpha.2', '0.164.0-alpha.10'])

    def test_missing_anchor_and_unpublished_install_are_errors(self):
        for last, installed in [('0.163.0', '0.164.0'), ('0.162.0', '0.163.0')]:
            with self.assertRaisesRegex(ValueError, 'missing baseline/installed'):
                watch.releases('codex', last, installed, lambda _: '[]')

    def test_no_upgrade_needs_no_release_fetch(self):
        with patch.object(watch, 'fetch', side_effect=AssertionError('network')) as fetch:
            self.assertEqual(watch.releases('codex', '0.164.0', '0.164.0', fetch), [])

    def test_downgrade_is_refused_before_fetch(self):
        f = fixture()
        f['versions']['claude'] = '2.1.8'
        with self.assertRaisesRegex(ValueError, 'refusing to rewind'):
            watch.collect(f['state'], lambda _: self.fail('network'), f['versions'], f['protocol'])

    def test_release_only_version_prints_its_own_section(self):
        f = fixture()
        f['responses'][watch.CHANGELOG] = '## 2.1.11\n- hooks'
        current = watch.collect(f['state'], f['responses'].__getitem__, f['versions'], f['protocol'])
        output = watch.digest(f['state'], f['before'], current)
        self.assertIn('### 2.1.10', output)
        self.assertIn('Release only; no CHANGELOG.md entry', output)

    def test_changelog_only_version_including_either_anchor(self):
        for absent in ('2.1.9', '2.1.10', '2.1.11'):
            with self.subTest(absent=absent):
                f = fixture()
                url = f'https://api.github.com/repos/{watch.REPOS["claude"]}/releases?per_page=100&page=1'
                f['responses'][url] = json.dumps([r for r in json.loads(f['responses'][url])
                                                 if r['tag_name'] != 'v'+absent])
                f['responses'][url.rsplit('&page=', 1)[0] + '&page=2'] = '[]'
                current = watch.collect(f['state'], f['responses'].__getitem__, f['versions'], f['protocol'])
                output = watch.digest(f['state'], f['before'], current)
                self.assertEqual([v for v, _ in current['notes']['claude']], ['2.1.10', '2.1.11'])
                if absent != '2.1.9':
                    self.assertIn(f'### {absent} (changelog only; no GitHub release)', output)

    def test_stable_changelog_comparison_excludes_preview_sections(self):
        f = fixture()
        f['responses'][watch.CHANGELOG] += '\n## 2.1.11-beta.1\n- Preview hooks\n'
        current = watch.collect(f['state'], f['responses'].__getitem__, f['versions'], f['protocol'])
        self.assertNotIn('Preview hooks', watch.digest(f['state'], f['before'], current))

    def test_source_only_changelog_entries_are_not_discarded(self):
        f = fixture()
        f['responses'][watch.CHANGELOG] = f['responses'][watch.CHANGELOG].replace(
            '## 2.1.11', '## 2.1.11\n- Only here: turn.step')
        current = watch.collect(f['state'], f['responses'].__getitem__, f['versions'], f['protocol'])
        self.assertIn('Only here: turn.step', watch.digest(f['state'], f['before'], current))

    def test_protocol_additions_removals_and_empty_generation(self):
        diff = watch.changes({'a.ts': 'old'}, {'b.ts': 'new'}, 'codex')
        self.assertIn('-old', diff)
        self.assertIn('+new', diff)
        with tempfile.TemporaryDirectory() as directory, self.assertRaisesRegex(ValueError, 'no TypeScript'):
            watch.protocol(Path(directory))


class CheckpointTests(unittest.TestCase):
    def test_normal_run_does_not_write_and_mark_read_checkpoints_complete_capture(self):
        f = fixture()
        for args in ([], ['--mark-read']):
            with self.subTest(args=args), patch.object(watch, 'load_checkpoint', return_value=(f['state'], f['before'])), \
                    patch.object(watch, 'installed_versions', return_value=f['versions']), \
                    patch.object(watch, 'run'), patch.object(watch, 'protocol', return_value=f['protocol']), \
                    patch.object(watch, 'fetch', side_effect=f['responses'].__getitem__), \
                    patch.object(watch, 'mark_read') as mark, \
                    contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(watch.main(args), 0)
                if args:
                    mark.assert_called_once_with(watch.STATE, watch.DATA, f['versions'], f['protocol'],
                                                 {n:f['responses'][u] for n,u in watch.SURFACES.items()})
                else:
                    mark.assert_not_called()

    def test_mark_read_saves_exact_generation_and_date_without_other_writes(self):
        f = fixture()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state_path, data = root/'release-watch.json', root/'release-watch'
            watch.mark_read(state_path, data, f['versions'], f['protocol'], {}, today=date(2026,10,24))
            state, snapshot = watch.load_checkpoint(state_path, data)
            self.assertEqual(state['claude'], {'version':'2.1.11','read_at':'2026-10-24'})
            self.assertEqual(snapshot['protocol'], f['protocol'])
            self.assertEqual(snapshot['versions'], f['versions'])
            self.assertEqual(len(list(root.rglob('*'))), 3)
            watch.mark_read(state_path, data, f['versions'], f['protocol'], {}, today=date(2026,10,25))
            self.assertEqual(watch.load_checkpoint(state_path, data)[0]['codex']['read_at'], '2026-10-25')
            self.assertEqual(len(list(data.iterdir())), 1, 'identical captures share one snapshot')

    def test_failed_capture_write_keeps_old_cursor(self):
        f = fixture()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            watch.mark_read(root/'state.json', root/'data', f['versions'], f['protocol'], {})
            old = (root/'state.json').read_bytes()
            with patch.object(watch, 'atomic_write', side_effect=OSError('disk full')):
                with self.assertRaises(OSError):
                    watch.mark_read(root/'state.json', root/'data', {'claude':'9.0.0','codex':'9.0.0'}, {}, {})
            self.assertEqual((root/'state.json').read_bytes(), old)

    def test_corrupt_capture_is_not_a_valid_baseline(self):
        f = fixture()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            watch.mark_read(root/'state.json', root/'data', f['versions'], f['protocol'], {})
            next((root/'data').iterdir()).write_text('{}')
            with self.assertRaisesRegex(ValueError, 'hash mismatch'):
                watch.load_checkpoint(root/'state.json', root/'data')

    def test_fetch_failure_and_version_race_never_mark_read(self):
        f = fixture()
        for fail in ('fetch', 'version race'):
            versions = [f['versions'], {**f['versions'], 'claude':'2.1.12'}]
            with self.subTest(fail=fail), patch.object(watch, 'load_checkpoint', return_value=(f['state'], f['before'])), \
                    patch.object(watch, 'installed_versions', side_effect=versions), \
                    patch.object(watch, 'run'), patch.object(watch, 'protocol', return_value=f['protocol']), \
                    patch.object(watch, 'fetch', side_effect=OSError('network down') if fail == 'fetch' else f['responses'].__getitem__), \
                    patch.object(watch, 'mark_read') as mark, contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(watch.main(['--mark-read']), 1)
                mark.assert_not_called()

    def test_dry_run_never_reads_binaries_network_or_real_checkpoint(self):
        with patch.object(watch, 'run', side_effect=AssertionError('binary')), \
                patch.object(watch, 'fetch', side_effect=AssertionError('network')), \
                patch.object(watch, 'load_checkpoint', side_effect=AssertionError('real state')), \
                patch.object(watch, 'mark_read', side_effect=AssertionError('write')), \
                contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(watch.main(['--dry-run']), 0)
            self.assertIn('synthetic offline fixtures', output.getvalue())
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            watch.main(['--dry-run', '--mark-read'])


class DeclarationTests(unittest.TestCase):
    def test_native_frame_extraction_and_unsupported_or_oversized_frames(self):
        try:
            from compression import zstd
        except ImportError:
            self.skipTest('native extraction requires Python 3.14')
        text = '// Claude Code function hooks: synthetic\ninterface Prompt {}\n'
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory)/'claude'
            binary.write_bytes(b'prefix' + zstd.compress(b'unrelated') + zstd.compress(text.encode()) + b'suffix')
            self.assertEqual(watch.claude_declarations(binary), text)
            for content, error in ((b'not a native binary', 'No embedded'),
                                   (zstd.compress((text + 'x'*2_000_000).encode()), 'extraction bound')):
                binary.write_bytes(content)
                with self.assertRaisesRegex(ValueError, error):
                    watch.claude_declarations(binary)

    def test_declaration_step_is_separate_and_only_marks_after_success(self):
        f = fixture()['claude_decls']
        with tempfile.TemporaryDirectory() as directory, patch.object(watch, 'DATA', Path(directory)), \
                patch.object(watch, 'fetch', side_effect=AssertionError('network')), \
                patch.object(watch, 'installed_versions', side_effect=AssertionError('Codex')), \
                patch.object(watch.shutil, 'which', return_value='/synthetic/claude'), \
                patch.object(watch, 'run', return_value=f['version']), \
                patch.object(watch, 'claude_declarations', return_value=f['text']) as extract, \
                contextlib.redirect_stdout(io.StringIO()) as output, contextlib.redirect_stderr(io.StringIO()):
            checkpoint = Path(directory)/'claude-decls.json'
            checkpoint.write_text(json.dumps(f['before']))
            old = checkpoint.read_bytes()
            self.assertEqual(watch.main(['--claude-decls']), 0)
            self.assertIn('+interface Turn', output.getvalue())
            self.assertEqual(checkpoint.read_bytes(), old)
            extract.side_effect = ValueError('missing declarations')
            self.assertEqual(watch.main(['--claude-decls', '--mark-read']), 1)
            self.assertEqual(checkpoint.read_bytes(), old)
            extract.side_effect = None
            self.assertEqual(watch.main(['--claude-decls', '--mark-read']), 0)
            self.assertEqual(json.loads(checkpoint.read_text())['text'], f['text'])
            extract.side_effect = AssertionError('binary')
            self.assertEqual(watch.main(['--claude-decls', '--dry-run']), 0)


if __name__ == '__main__':
    unittest.main()

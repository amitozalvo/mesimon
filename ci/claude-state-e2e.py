#!/usr/bin/env python3
"""Opt-in paid Claude/Mesimon verification. Run through ci/test-run.py.

Production spawning, generated observer hooks, private tmux and real snapshots.
Extra capture hooks collect evidence only; they never forward fabricated events.
"""
import argparse
import datetime
import importlib.util
import hashlib
import json
import os
import platform
import re
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import time
import uuid
from state_lab_faults import FaultServer, ERRORS

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('state_lab', ROOT / 'ci/state-lab.py')
lab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lab)
EVENTS = ['SessionStart', 'SessionEnd', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse',
          'PostToolUseFailure', 'PermissionRequest', 'PermissionDenied', 'Stop', 'StopFailure',
          'PreCompact', 'PostCompact', 'SubagentStart', 'SubagentStop', 'TeammateIdle',
          'Elicitation', 'ElicitationResult', 'Notification']
CASES = ['complete', 'permission-allow', 'permission-deny', 'permission-cancel',
         'interrupt-tool', 'interrupt-early', 'interrupt-stream', 'question', 'question-cancel', 'plan', 'plan-cancel', 'background-shell',
         'compact', 'auto-compact', 'cron-wakeup', 'loop-wakeup', 'monitor-wakeup',
         'clear', 'resume', 'elicitation-accept', 'elicitation-decline',
         'elicitation-cancel', 'teammate', *ERRORS]


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def lines(path):
    try:
        text = path.read_text()
    except FileNotFoundError:
        return []
    result = []
    for line in text.splitlines():
        try:
            result.append(json.loads(line))
        except ValueError:
            pass  # A concurrent append can end in a partial final line.
    return result


class Prerequisite(Exception):
    pass


class Guard:
    def __init__(self, timeout):
        self.process = subprocess.Popen([sys.executable, '-B', str(ROOT / 'ci/test_guard.py'),
            '--name', 'claude-live', '--tmux', shutil.which('tmux'), '--timeout', str(timeout)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.root = Path(json.loads(self.process.stdout.readline())['ok'])

    def request(self, **value):
        self.process.stdin.write(json.dumps(value) + '\n')
        self.process.stdin.flush()
        reply = json.loads(self.process.stdout.readline())
        if 'error' in reply:
            raise RuntimeError(reply['error'])
        return reply['ok']

    def close(self):
        self.process.stdin.write('{"op":"finish"}\n')
        self.process.stdin.flush()
        self.process.stdin.close()
        return self.process.wait(timeout=20) == 0


def launch(config_path, arguments):
    """Called only by the lab daemon in place of the Claude executable."""
    config = json.loads(Path(config_path).read_text())
    out = Path(config['out'])
    index = arguments.index('--settings') + 1
    original = json.loads(Path(arguments[index]).read_text())
    write(out / 'production-settings.json', original)
    original['plansDirectory'] = str(Path(config['repo']) / '.claude' / 'plans')
    for event in EVENTS:
        command = shlex.join([sys.executable, '-B', str(ROOT / 'ci/claude-capture.py'),
                             'hook', '--log', str(out / 'hooks.jsonl'), '--event', event])
        original['hooks'].setdefault(event, []).append({'hooks': [
            {'type': 'command', 'command': command, 'timeout': 5}]})
    if config['case'] == 'teammate':
        original['hooks'].setdefault('PreToolUse', []).append({'matcher':'Agent', 'hooks':[{
            'type':'command', 'command':shlex.join([sys.executable,'-B',str(Path(__file__).resolve()),'gate-model']), 'timeout':5}]})
    write(out / 'settings.json', original)
    arguments[index] = str(out / 'settings.json')
    tools = config['tools']
    argv = [config['claude'], *arguments, '--model', config['model'], '--setting-sources', '',
            '--no-chrome', '--strict-mcp-config', '--mcp-config', json.dumps(config.get('mcp', {'mcpServers': {}})),
            '--tools', tools, '--permission-mode', 'plan' if config['case'].startswith('plan') else 'default']
    if config['case'] in ('teammate','background-shell','auto-compact','cron-wakeup','loop-wakeup','monitor-wakeup'):
        argv.extend(['--allowedTools', 'Bash(*)'])
    if config['case'].startswith('elicitation'):
        argv.extend(['--allowedTools', 'mcp__lab__ask'])
    if config['case'] == 'auto-compact':
        argv.extend(['--autocompact', '100000', '--debug-file', str(out/'claude-debug.log')])
    write(out / 'argv.json', argv)
    env = {k: v for k, v in os.environ.items() if k not in
           ('CLAUDECODE', 'CLAUDE_CODE_CHILD_SESSION', 'CLAUDE_CODE_ENTRYPOINT')}
    env['CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS'] = '1' if config['case'] == 'teammate' else '0'
    env['DISABLE_AUTOUPDATER'] = '1'
    if config['case'] == 'auto-compact':
        env.pop('CLAUDE_AUTOCOMPACT_PCT_OVERRIDE', None)
        env['CLAUDE_CODE_AUTO_COMPACT_WINDOW'] = '100000'
    if 'fault_url' in config:
        env = {k:v for k,v in env.items() if not k.startswith('ANTHROPIC_')
               and not k.startswith('CLAUDE_CODE_USE_') and k != 'CLAUDE_CODE_OAUTH_TOKEN'}
        env.update(ANTHROPIC_BASE_URL=config['fault_url'], ANTHROPIC_AUTH_TOKEN='lab-placeholder',
                   CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1')
    os.execve(config['claude'], argv, env)


class Probe:
    def __init__(self, args, out, guard, manifest):
        self.args, self.out, self.guard, self.manifest = args, out, guard, manifest
        self.fault = None
        self.repo = guard.root / 'repo'
        self.repo.mkdir()
        subprocess.run(['git', 'init', '-q', str(self.repo)], check=True, timeout=10)
        (self.repo / 'README.md').write_text('Disposable Claude state verification repository.\n')
        self.runtime, self.state = lab.paths(self.repo)
        guard.request(op='register', repo=str(self.repo), state=str(self.state),
                      runtime=str(self.runtime), sock=str(self.runtime / 'tmux.sock'))
        self.binary = args.binary.resolve()
        tools = {'question': 'AskUserQuestion', 'plan': 'Read,Write,ExitPlanMode',
                 'auto-compact': 'Read,Bash',
                 'cron-wakeup': 'CronCreate,CronList,CronDelete,Bash',
                 'loop-wakeup': 'ScheduleWakeup,Bash',
                 'monitor-wakeup': 'Monitor,TaskStop,Bash',
                 'teammate': 'Agent,SendMessage,Bash,TaskOutput'}.get(args.case.removesuffix('-cancel'), 'Bash')
        if args.case in ('complete', 'interrupt-early', 'interrupt-stream', 'compact', 'clear', 'resume'):
            tools = ''
        config = dict(out=str(out), repo=str(self.repo), claude=str(args.claude_binary), model=args.model, tools=tools, case=args.case)
        if args.case == 'auto-compact':
            manifest['autocompact_window_tokens'] = 100000
            # Bounded generated input, never copied from a user's conversation.
            for number in range(6):
                (self.repo/f'context-{number}.txt').write_text(
                    ('alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima\n')*800)
        if args.case.startswith('elicitation'):
            config['mcp'] = {'mcpServers': {'lab': {'command': sys.executable,
                'args': ['-B', str(ROOT / 'ci/state-lab-mcp.py'), str(out / 'mcp.jsonl')]}}}
        if args.case in ERRORS:
            self.fault = FaultServer(args.case, out/'fault-requests.jsonl')
            config['fault_url'] = self.fault.url
            manifest['kind'] = 'injected_api_real_claude_mesimon_e2e'
            manifest['limitation'] = 'Local API error injection; no account quota exhaustion or upstream model call'
        write(out / 'launch.json', config)
        wrapper = guard.root / 'claude-wrapper'
        wrapper.write_text('#!/bin/sh\nexec ' + shlex.join([sys.executable, '-B', str(Path(__file__).resolve()),
                          'launch', str(out / 'launch.json')]) + ' "$@"\n')
        wrapper.chmod(0o700)
        env = {k: v for k, v in os.environ.items() if not k.startswith('MESIMON_') and k not in ('TMUX', 'TMUX_PANE')}
        env.update(MESIMON_CLAUDE_BIN=str(wrapper), MESIMON_HOOK_BIN=str(self.binary),
                   MESIMON_TMUX_BIN=shutil.which('tmux'), MESIMON_NO_DAEMON_AUTORESTART='1')
        # Use real matching session files for status inference, never a synthetic CLAUDE_HOME.
        guard.request(op='spawn', argv=[str(self.binary), 'daemon', '--repo', str(self.repo)], env=env)
        lab.wait_for(lambda: (self.runtime / 'orch.sock').exists())
        self.client = lab.Client(self.repo)
        self.client.request('set_mcp_tools', on=False)
        self.ticket = self.client.request('create_ticket', column='IN PROGRESS', title='State lab', workspace=None)['id']
        self.sid = self.client.request('spawn_session', ticket=self.ticket, kind='claude', submit_prompt=False)['id']
        self.target = self.sid.replace('-', '')[:16]
        manifest.update(session_id=self.sid, repo=str(self.repo), mesimon_version=subprocess.check_output(
            [str(self.binary), '--version'], text=True).strip())
        with self.binary.open('rb') as artifact:
            manifest['mesimon_sha256'] = hashlib.file_digest(artifact, 'sha256').hexdigest()
        self.started = time.monotonic()
        self.screen, self.row, self.events = '', {}, []
        self.stage = 'startup'
        self.previous_screen = self.previous_row = None
        self.seen_status = set()
        self.timeline = (out / 'timeline.jsonl').open('w')
        self.terminal = (out / 'terminal.jsonl').open('w')
        self.statuses = (out / 'status-files.jsonl').open('w')
        self.last_status_poll = 0

    def tm(self, *args):
        result = subprocess.run([shutil.which('tmux'), '-S', str(self.runtime / 'tmux.sock'), *args],
                                capture_output=True, text=True, timeout=5)
        if result.returncode:
            raise Prerequisite('tmux: ' + result.stderr.strip())
        return result.stdout

    def key(self, *keys):
        self.tm('send-keys', '-t', self.target, *keys)
        self.mark(action='keys', keys=list(keys))

    def send(self, text):
        self.stop_count_at_submit = len(self.stops())
        self.key('C-e', *(['C-w'] * 80), 'C-u')
        self.tm('send-keys', '-t', self.target, '-l', text)
        self.hold(0.5)
        self.key('Enter')
        self.mark(action='submit', text=text)

    def mark(self, **value):
        self.manifest['checkpoints'].append(dict(at_ms=int((time.monotonic()-self.started)*1000), stage=self.stage, **value))

    def pump(self):
        elapsed = time.monotonic() - self.started
        if elapsed > self.args.timeout:
            raise Prerequisite('whole scenario deadline exceeded')
        self.screen = self.tm('capture-pane', '-p', '-t', self.target)
        board = self.client.board()
        session = next(s for s in board['sessions'] if s['id'] == self.sid)
        ticket = next(t for t in board['tickets'] if t['id'] == self.ticket)
        state = session['state']
        self.row = dict(state=state['state'], reason=state.get('reason', state.get('stop_reason')),
                        column=ticket['column'], confidence=session['confidence'])
        stamp = dict(at_ms=int(elapsed*1000), stage=self.stage)
        if self.row != self.previous_row:
            self.timeline.write(json.dumps(dict(**stamp, **self.row))+'\n'); self.timeline.flush()
            self.previous_row = self.row.copy()
        if self.screen != self.previous_screen:
            self.terminal.write(json.dumps(dict(**stamp, screen=self.screen))+'\n'); self.terminal.flush()
            self.previous_screen = self.screen
        self.events = lines(self.out / 'hooks.jsonl')
        # SessionStart's identity-checked path is the only transcript we read.
        for event in self.events:
            p = event.get('payload', {})
            if event['event'] == 'SessionStart' and p.get('session_id') == self.sid:
                path = Path(p.get('transcript_path', ''))
                if path.is_file() and path.stem == self.sid:
                    shutil.copyfile(path, self.out / 'transcript.jsonl')
                break
        if elapsed - self.last_status_poll > 0.4:
            self.last_status_poll = elapsed
            pane_pid = int(self.tm('display-message', '-p', '-t', self.target, '#{pane_pid}').strip())
            table = subprocess.check_output(['ps', '-axo', 'pid=,ppid='], text=True, timeout=5)
            pairs = [tuple(map(int, line.split())) for line in table.splitlines() if len(line.split()) == 2]
            pids = {pane_pid}
            for _ in range(8):
                pids.update(pid for pid, parent in pairs if parent in pids)
            home = Path(os.environ.get('CLAUDE_CONFIG_DIR', str(Path.home() / '.claude')))
            for pid in pids:
                try:
                    value = json.loads((home / 'sessions' / f'{pid}.json').read_text())
                except (OSError, ValueError):
                    continue
                if value.get('sessionId') == self.sid and (signature := json.dumps(value, sort_keys=True)) not in self.seen_status:
                    self.seen_status.add(signature)
                    self.statuses.write(json.dumps(dict(**stamp, value=value))+'\n'); self.statuses.flush()
        return self.row

    def wait(self, label, predicate, timeout=25):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.pump()
            if predicate():
                self.mark(observation=label)
                return
            time.sleep(0.1)
        raise Prerequisite('checkpoint not reached: ' + label)

    def hold(self, seconds, forbidden_review=False):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            self.pump()
            if forbidden_review:
                assert self.row['column'] != 'REVIEW', 'card moved to REVIEW during ' + self.stage
            time.sleep(0.1)

    def expect(self, state, reason=None, column='IN PROGRESS', timeout=8):
        expected = dict(state=state, reason=reason, column=column)
        try:
            self.wait('Mesimon checkpoint', lambda: all(self.row[k] == v for k, v in expected.items()), timeout)
        except Prerequisite as error:
            raise AssertionError(f'{self.stage}: expected {expected}, got {self.row}') from error
        self.mark(assertion=expected, passed=True)

    def stops(self):
        return [r for r in self.events if r['event']=='Stop' and not r.get('payload',{}).get('agent_id')]

    def has(self, event):
        return any(r['event'] == event for r in self.events)

    def ready(self):
        last_key = 0
        end = time.monotonic() + 25
        while time.monotonic() < end:
            self.pump()
            if self.has('SessionStart') and 'Yes, I trust this folder' not in self.screen:
                shown = re.search(r'Claude Code v([\d.]+)', self.screen)
                if shown:
                    self.manifest['displayed_claude_version'] = shown.group(1)
                    assert shown.group(1) == self.manifest['claude_version'].split()[0], 'launched Claude version differs from recorded version'
                self.hold(1)
                self.key('C-u')
                self.expect('idle', 'unknown')
                return
            elapsed = time.monotonic() - self.started
            if elapsed > 1.5 and elapsed - last_key > 0.7:
                if '❯ No, exit' in self.screen:
                    self.key('Down'); last_key = elapsed
                elif '❯ Yes, I trust this folder' in self.screen:
                    self.key('Enter'); last_key = elapsed
            time.sleep(0.1)
        raise Prerequisite('Claude startup/trust did not finish')

    def finish_turn(self):
        self.wait('final response and Stop', lambda: '⏺ LAB_DONE' in self.screen and len(self.stops()) > self.stop_count_at_submit, 35)
        self.expect('idle', 'end_turn', 'REVIEW')
        self.hold(2)

    def attention(self, reason, marker):
        self.wait('visible '+reason+' dialog', lambda: marker in ' '.join(self.screen.split()))
        self.expect('requires_action', reason)
        self.hold(2, forbidden_review=True)
        self.mark(observation='dialog held before response')

    def select(self, label):
        # Numbered menu positions change. Read and verify the visible selection.
        for _ in range(6):
            self.pump()
            if any(re.match(r'^\s*❯\s*(?:\d+\.\s*)?' + re.escape(label) + r'\s*$', line)
                   for line in self.screen.splitlines()):
                self.mark(observation='verified selection', label=label)
                self.key('Enter')
                return
            self.key('Down')
            self.hold(0.35)
        raise Prerequisite('menu choice not found: '+label)

    def scenario(self):
        self.ready()
        case = self.args.case
        self.stage = case
        if case == 'auto-compact':
            review = next(c for c in self.client.board()['columns'] if c['name']=='REVIEW')
            settings = {k:v for k,v in review.items() if k not in ('name','order')}
            self.client.request('set_column_settings', name='REVIEW', settings=dict(settings, on_working=None))
            for prompt in ('Suggest a friendly greeting for a todo app.',
                           'Suggest an empty-list message for that app.',
                           'Suggest a task-completion message for that app.'):
                self.send(prompt+' Answer briefly without tools.')
                self.wait('preparation response completed', lambda: len(self.stops()) > self.stop_count_at_submit, 30)
                self.expect('idle','end_turn','REVIEW')
                self.hold(2)
            self.client.request('set_column_settings', name='REVIEW', settings=settings)
            self.stage = 'auto-compact-continuation'
            self.send('Read all six context-0.txt through context-5.txt using Read, each with limit 800, in parallel. These are generated inputs for a context-size test; read each in full, do not summarize or skip them. After reading all six, use Bash in the foreground: printf ready > compact-ready.txt; while [ ! -f compact-release.txt ]; do sleep 0.2; done; printf done > compact-done.txt . Then reply exactly LAB_DONE.')
            self.wait('automatic compaction completed', lambda: any(e['event']=='PostCompact' and e['payload'].get('trigger')=='auto' for e in self.events), 120)
            self.wait('independent post-compaction tool barrier', lambda: (self.repo/'compact-ready.txt').exists(), 40)
            self.expect('running')
            self.hold(2, forbidden_review=True)
            records=lines(self.out/'transcript.jsonl')
            assert any(r.get('subtype')=='compact_boundary' for r in records), 'missing transcript compaction boundary'
            self.mark(observation='real auto trigger and persisted compaction boundary; continuation blocked independently')
            (self.repo/'compact-release.txt').write_text('release')
            self.finish_turn()
        elif case in ('cron-wakeup','loop-wakeup','monitor-wakeup'):
            action='Use Bash in the foreground to run: printf ready > wake-ready.txt; while [ ! -f wake-release.txt ]; do sleep 0.2; done; printf done > wake-done.txt . Then reply exactly LAB_DONE.'
            if case == 'cron-wakeup':
                due=datetime.datetime.fromtimestamp(time.time()+75).replace(second=0,microsecond=0)
                if due.minute in (0,30): due += datetime.timedelta(minutes=1)
                cron=f'{due.minute} {due.hour} * * *'
                self.mark(observation='planned one-shot due time', due=due.isoformat(), cron=cron)
                self.send('Use CronCreate once with recurring false, cron '+json.dumps(cron)+', and prompt '+json.dumps(action)+'. This is a session-only task, not persistent. After scheduling reply exactly LAB_ARMED and stop your turn. Do not run the task now.')
                tool='CronCreate'
            elif case == 'loop-wakeup':
                self.send('/loop For this lab only: on the first iteration call ScheduleWakeup for one minute from now and reply LAB_ARMED without other tools. On the second iteration call ScheduleWakeup with stop true, then '+action+' Do not schedule a third iteration. Do not use Monitor or CronCreate.')
                tool='ScheduleWakeup'
            else:
                command='printf ready > monitor-ready.txt; while [ ! -f monitor-release.txt ]; do sleep 0.2; done; printf "LAB_MONITOR_EVENT\\n"; while [ ! -f monitor-stop.txt ]; do sleep 0.2; done'
                self.send('Use Monitor once with command '+json.dumps(command)+' and persistent true. After arming it reply exactly LAB_ARMED and end your turn. When LAB_MONITOR_EVENT arrives, '+action+' Do not poll or run the monitor command through Bash.')
                tool='Monitor'
                self.attention('permission', 'Do you want to proceed?')
                self.key('Enter')
            self.wait('real scheduler/watch tool completion', lambda: any(e['event']=='PostToolUse' and e['payload'].get('tool_name')==tool for e in self.events), 35)
            self.wait('parent yielded with armed future work', lambda: 'LAB_ARMED' in self.screen and bool(self.stops()))
            self.expect('idle','end_turn','REVIEW')
            self.hold(2)
            assert not (self.repo/'wake-ready.txt').exists(), 'scheduled work ran before idle checkpoint'
            self.stage=case+'-automatic-fire'
            stop_count=len(self.stops())
            submits=sum(e['event']=='UserPromptSubmit' for e in self.events)
            if case=='monitor-wakeup':
                assert (self.repo/'monitor-ready.txt').exists(), 'monitor script never armed'
                (self.repo/'monitor-release.txt').write_text('release')
            self.wait('independent automatic wake tool barrier', lambda: (self.repo/'wake-ready.txt').exists(), 150)
            self.expect('running')
            self.hold(2, forbidden_review=True)
            self.mark(observation='automatic fire without driver submission', submit_hooks_before=submits,
                      submit_hooks_after=sum(e['event']=='UserPromptSubmit' for e in self.events))
            (self.repo/'wake-release.txt').write_text('release')
            self.stop_count_at_submit=stop_count
            self.finish_turn()
            assert (self.repo/'wake-done.txt').exists(), 'wake continuation did not complete'
            if case in ('cron-wakeup','loop-wakeup'):
                assert not self.stops()[-1]['payload'].get('session_crons'), 'one-shot/loop still scheduled after completion'
                self.mark(observation='no pending wakeups in final Stop')
            if case=='monitor-wakeup': (self.repo/'monitor-stop.txt').write_text('stop')
        elif case in ('complete', 'compact', 'clear', 'resume'):
            self.send('Reply exactly LAB_DONE. Do not use tools.')
            self.finish_turn()
            if case != 'complete':
                if case == 'compact':
                    review = next(c for c in self.client.board()['columns'] if c['name']=='REVIEW')
                    settings = {k:v for k,v in review.items() if k not in ('name','order')}
                    self.mark(action='setup column rule', reason='Avoid six-move fuse during context preparation')
                    self.client.request('set_column_settings', name='REVIEW', settings=dict(settings, on_working=None))
                    for number in range(3):
                        self.send('This is text-only lab turn '+str(number)+'. Reply exactly LAB_DONE. Do not use tools or save anything.')
                        self.finish_turn()
                    self.client.request('set_column_settings', name='REVIEW', settings=settings)
                self.stage = case + '-action'
                self.send('/' + (('resume '+self.sid) if case == 'resume' else case))
                if case == 'compact':
                    self.wait('compaction completed', lambda: self.has('PreCompact') and self.has('PostCompact'), 45)
                    self.expect('idle','end_turn','REVIEW')
                else:
                    self.wait('new SessionStart', lambda: sum(r['event']=='SessionStart' for r in self.events) >= 2)
                self.hold(2)
                assert self.row['state'] != 'exited', 'conversation operation killed Mesimon session state'
                self.stage = case + '-recovery'
                self.send('Reply exactly LAB_DONE. Do not use tools.')
                self.wait('new prompt reached daemon', lambda: self.row['state'] == 'running', 8)
                self.finish_turn()
        elif case in ERRORS:
            self.send('Reply LAB_DONE.')
            self.wait('local API request received', lambda: bool(lines(self.out/'fault-requests.jsonl')))
            self.wait('Claude displayed injected API failure', lambda: 'LAB_INJECTED_' in self.screen or (case == 'fault-model' and 'issue with the selected model' in self.screen), 65)
            expected = {'fault-rate-limit': ('throttled',None), 'fault-auth': ('requires_action','auth'),
                        'fault-server': ('failed','server'), 'fault-model': ('failed','model_not_found')}[case]
            self.expect(*expected)
            self.hold(2, forbidden_review=True)
        elif case.startswith('permission') or case == 'interrupt-tool':
            command = 'printf ready > tool-started.txt; sleep 60' if case == 'interrupt-tool' else 'printf hello > greeting.txt'
            self.send('For this temporary lab, use Bash to run '+command+'. Run it in the foreground. If rejected, do not retry. Then reply LAB_DONE.')
            self.attention('permission', 'Do you want to proceed?')
            if case == 'permission-deny':
                self.select('No')
            elif case == 'permission-cancel':
                self.key('Escape')
            else:
                self.key('Enter')
            self.stage = case + '-response'
            if case in ('permission-deny', 'permission-cancel', 'interrupt-tool'):
                if case == 'interrupt-tool':
                    self.wait('independent tool-start file', lambda: (self.repo/'tool-started.txt').exists())
                    self.expect('running')
                    self.hold(2, forbidden_review=True)
                    self.key('Escape')
                self.wait('visible interrupted response', lambda: 'Interrupted' in self.screen or 'interrupted' in self.screen
                    or 'User declined to answer' in self.screen or "User rejected Claude's plan" in self.screen)
                self.expect('idle', 'interrupted')
                self.hold(2, forbidden_review=True)
                assert not (self.repo/'greeting.txt').exists(), 'rejected tool wrote a file'
                self.recover()
            else:
                self.finish_turn()
                exists = (self.repo/'greeting.txt').exists()
                assert exists == (case == 'permission-allow'), 'file execution disagrees with permission response'
        elif case.startswith('interrupt-'):
            self.send('Write the integers from 1 to 300, one per line. Start immediately. Do not use tools.')
            self.wait('prompt submitted', lambda: self.has('UserPromptSubmit'))
            if case == 'interrupt-stream':
                self.wait('visible generated sequence', lambda: bool(re.search(r'(?m)^\s*\d+\s*$', self.screen)))
            else:
                self.hold(self.args.early_delay_ms / 1000)
                records = lines(self.out / 'transcript.jsonl')
                if any(r.get('type') == 'assistant' for r in records):
                    raise Prerequisite('early interrupt missed: assistant output already recorded')
                self.mark(observation='no assistant transcript record before Escape')
            self.key('Escape')
            self.wait('visible interruption or restored unsubmitted prompt', lambda:
                'Interrupted' in self.screen or 'interrupted' in self.screen or
                (case == 'interrupt-early' and '❯\u00a0Write the integers' in self.screen))
            self.expect('idle', 'interrupted')
            self.hold(2, forbidden_review=True)
            self.recover()
        elif case.startswith('question'):
            self.send('Use AskUserQuestion to ask which color I prefer: Blue or Green. Wait for my answer, then reply LAB_DONE. Do not guess.')
            self.attention('question', 'Enter to select')
            if case == 'question-cancel':
                self.key('Escape')
                self.wait('visible cancellation', lambda: 'Interrupted' in self.screen or 'interrupted' in self.screen
                    or 'User declined to answer' in self.screen or "User rejected Claude's plan" in self.screen)
                self.expect('idle','interrupted')
                self.recover()
            else:
                self.key('Enter')
                self.finish_turn()
        elif case.startswith('plan'):
            self.send('Plan a one-line greeting.txt file containing hello. This is only a plan: write the plan file and use ExitPlanMode for approval. After approval reply LAB_DONE without implementing it.')
            self.attention('plan', 'Would you like to proceed?')
            if case == 'plan-cancel':
                self.key('Escape')
                self.wait('visible cancellation', lambda: 'Interrupted' in self.screen or 'interrupted' in self.screen
                    or 'User declined to answer' in self.screen or "User rejected Claude's plan" in self.screen)
                self.expect('idle','interrupted')
                self.recover()
            else:
                self.select('Yes, manually approve edits')
                self.finish_turn()
        elif case.startswith('elicitation'):
            self.send('Call the lab MCP ask tool once. After my response, regardless of the action, reply LAB_DONE. Do not call it twice.')
            self.attention('elicitation', 'LAB_ELICIT')
            if case == 'elicitation-cancel':
                self.key('Escape')
            else:
                self.key('Enter')  # Confirm the default field value first.
                self.wait('form action buttons focused', lambda: '❯ Accept' in self.screen)
                if case == 'elicitation-decline':
                    self.key('Right')
                    self.wait('Decline selected', lambda: '❯ Decline' in self.screen)
                self.key('Enter')
            self.wait('MCP received response', lambda: any(r['direction']=='in' and 'result' in r['message']
                and str(r['message'].get('id','')).startswith('lab-elicitation-') for r in lines(self.out/'mcp.jsonl')))
            responses=[r['message']['result'] for r in lines(self.out/'mcp.jsonl') if r['direction']=='in'
                and 'result' in r['message'] and str(r['message'].get('id','')).startswith('lab-elicitation-')]
            assert responses[-1]['action'] == case.removeprefix('elicitation-'), responses[-1]
            self.finish_turn()
        elif case == 'background-shell':
            self.send('Use Bash with run_in_background true for: printf ready > background-ready.txt; while [ ! -f background-release.txt ]; do sleep 0.2; done; printf done > background-done.txt '
                'Then reply LAB_WAITING immediately. Do not wait for or stop the background task.')
            self.wait('background shell started', lambda: (self.repo/'background-ready.txt').exists())
            self.wait('parent yielded while shell remains blocked', lambda: 'LAB_WAITING' in self.screen and bool(self.stops()))
            self.expect('idle','background')
            self.hold(2,forbidden_review=True)
            (self.repo/'background-release.txt').write_text('release')
            self.wait('background shell completed', lambda: (self.repo/'background-done.txt').exists())
            self.send('The background shell finished. Reply exactly LAB_DONE. Do not use tools.')
            self.finish_turn()
        elif case == 'teammate':
            self.send('Test one agent teammate. Use Agent with name labworker, model haiku, and run_in_background true. '
                'Tell that worker to run in Bash: printf ready > teammate-ready.txt; while [ ! -f teammate-release.txt ]; do sleep 0.2; done; printf done > teammate-done.txt '
                'and then reply WORKER_DONE. Spawn exactly one named teammate. Do not run the shell yourself. '
                'After spawning, reply LAB_WAITING immediately, without waiting for the worker.')
            self.wait('independent teammate tool barrier', lambda: (self.repo/'teammate-ready.txt').exists(), 45)
            self.wait('parent yielded while worker remains blocked', lambda: 'LAB_WAITING' in self.screen and self.has('Stop'))
            self.expect('idle','background')
            self.hold(2, forbidden_review=True)
            self.mark(action='release teammate tool')
            (self.repo/'teammate-release.txt').write_text('release')
            self.wait('independent teammate tool completion', lambda: (self.repo/'teammate-done.txt').exists())
            self.wait('real teammate idle event', lambda: self.has('TeammateIdle'), 35)
            self.send('The lab worker has finished. Reply exactly LAB_DONE. Do not spawn any more agents.')
            self.finish_turn()

    def recover(self):
        self.stage = self.args.case + '-recovery'
        self.send('Reply exactly LAB_DONE. Do not use tools.')
        self.expect('running')
        self.finish_turn()

    def save(self):
        try: self.pump()
        except Exception as error: self.manifest['last_poll_error'] = str(error)
        self.manifest['resolved_models'] = sorted({r['message']['model'] for r in lines(self.out/'transcript.jsonl')
            if isinstance(r.get('message'),dict) and r['message'].get('model')})
        self.manifest['events_before_cleanup'] = [r['event'] for r in self.events]
        self.timeline.close(); self.terminal.close(); self.statuses.close()
        for name in ('activity.jsonl', 'activity.jsonl.1'):
            source = self.state / name
            if source.exists(): shutil.copyfile(source, self.out / name)
        plans = self.repo / '.claude' / 'plans'
        if plans.exists(): shutil.copytree(plans, self.out / 'plans', dirs_exist_ok=True)
        self.client.close()
        if self.fault: self.fault.close()


def main():
    if len(sys.argv)>1 and sys.argv[1] == 'gate-model':
        payload = json.load(sys.stdin)
        if payload.get('tool_input',{}).get('model') not in ('haiku','sonnet'):
            print(json.dumps({'hookSpecificOutput':{'hookEventName':'PreToolUse','permissionDecision':'deny',
                'permissionDecisionReason':'This lab requires an explicit haiku or sonnet model for its single worker.'}}))
        return 0
    if len(sys.argv)>1 and sys.argv[1] == 'launch':
        launch(sys.argv[2], sys.argv[3:])
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--quiet', action='store_true')
    parser.add_argument('--case', choices=CASES, required=True)
    parser.add_argument('--model', choices=['haiku', 'sonnet'], default='haiku')
    parser.add_argument('--timeout', type=int, default=None)
    parser.add_argument('--early-delay-ms', type=int, choices=[0, 150, 500], default=0)
    parser.add_argument('--binary', type=Path, default=ROOT/'target/debug/mesimon')
    parser.add_argument('--claude-binary', type=Path, help='pin an installed Claude executable (default: resolve PATH once)')
    args = parser.parse_args()
    if args.timeout is None:
        args.timeout = 180 if args.case in ('auto-compact','cron-wakeup','loop-wakeup','monitor-wakeup') else 100
    if not os.environ.get('MESIMON_TEST_RUN'):
        parser.error('run via python3 -B ci/test-run.py -- python3 -B ci/claude-state-e2e.py ...')
    if not 30 <= args.timeout <= 180:
        parser.error('timeout must be 30..180 seconds')
    executable = args.claude_binary or shutil.which('claude')
    if not executable:
        parser.error('Claude executable not found')
    args.claude_binary = Path(executable).resolve(strict=True)
    os.umask(0o077)
    out = ROOT/'target/state-lab/captures'/(datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')+'-e2e-'+args.case+'-'+uuid.uuid4().hex[:6])
    out.mkdir(parents=True)
    manifest = dict(schema=3, kind='real_claude_mesimon_e2e', case=args.case, model=args.model,
        claude_version=subprocess.check_output([str(args.claude_binary),'--version'],text=True,
            env=dict(os.environ, DISABLE_AUTOUPDATER='1')).strip(), result='inconclusive',
        claude_binary=str(args.claude_binary), claude_sha256=hashlib.sha256(args.claude_binary.read_bytes()).hexdigest(),
        runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), auto_updater_disabled=True,
        os=platform.platform(), tmux_version=subprocess.check_output(['tmux','-V'],text=True).strip(),
        checkpoints=[], limitation='Interactive wall-clock cap only; extra non-deciding capture hooks add overhead',
        early_delay_ms=args.early_delay_ms)
    write(out/'manifest.json', manifest)
    guard, probe = Guard(args.timeout+50), None
    try:
        probe = Probe(args, out, guard, manifest)
        probe.scenario()
        manifest['result'] = 'passed'
    except AssertionError as error:
        manifest.update(result='failed', error=str(error))
    except Exception as error:
        manifest['error'] = str(error)
    finally:
        if probe:
            try: probe.save()
            except Exception as error: manifest['save_error'] = str(error)
        manifest['cleanup'] = 'cleaned' if guard.close() else 'failed'
        if manifest['cleanup'] != 'cleaned': manifest['result'] = 'cleanup_failed'
        write(out/'manifest.json', manifest)
    result = dict(capture=str(out), **manifest)
    if args.quiet: result = {k:result.get(k) for k in ('capture','case','result','error','cleanup')}
    print(json.dumps(result, indent=2))
    return 0 if manifest['result']=='passed' else 1


if __name__ == '__main__':
    sys.exit(main())

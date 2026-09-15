#!/usr/bin/env python3
import json,os,select,signal,sys,time,tty
from pathlib import Path
config=json.loads(Path(sys.argv[sys.argv.index('--config')+1]).read_text())
root=Path(__file__).parent
with (root/'runtime-launches.jsonl').open('a') as log:
    log.write(json.dumps(config)+'\n')
path=Path(config['snapshot_path']); control=Path(str(path)+'.control')
thread=config['resume'] or ('synthetic-'+config['session'])
state={'state':'idle','stop_reason':'unknown'}; turn=None; sequence=1
prior=None; submits=0; pending=b''; pasted=False; stopping=False
def stop(*_):
    global stopping
    stopping=True
signal.signal(signal.SIGTERM,stop)
tty.setcbreak(0)
def render(screen,cursor):
    print('\x1b[?25l\x1b[2J\x1b[3J\x1b[H'+screen,end='')
    if cursor is not None:
        x,y=cursor
        print(f'\x1b[{y+1};{x+1}H\x1b[?25h',end='')
    sys.stdout.flush()
print('\x1b[?2004h',end='')
try: initial=json.loads((root/'initial-screen.json').read_text())
except FileNotFoundError: initial={}
render(initial.get('screen','Synthetic provider fixture\r\n› \r\nanything the user wants'),initial.get('cursor',[2,1]))
while not stopping:
    try: instruction=json.loads(control.read_text())
    except (FileNotFoundError,ValueError): instruction={}
    if instruction!=prior:
        if 'state' in instruction: state=instruction['state']
        if 'turn_id' in instruction: turn=instruction['turn_id']
        if 'screen' in instruction: render(instruction['screen'],instruction.get('cursor',[2,1]))
        sequence+=1; prior=instruction
    if instruction.get('publish',True):
        value={'session':config['session'],'generation':config['generation'],'sequence':sequence,
               'heartbeat_ms':time.time_ns()//1000000,'thread_id':thread,'turn_id':turn,
               'state':state,'observation_hold':instruction.get('hold',False),'history_path':None,'stopped':False}
        temporary=path.with_suffix('.tmp')
        temporary.write_text(json.dumps(value));temporary.replace(path)
    if select.select([sys.stdin],[],[],0.1)[0]:
        data=os.read(0,65536)
        if not data: break
        with (root/(config['session']+'.input')).open('ab') as log: log.write(data)
        pending+=data
        while pending:
            if pending.startswith(b'\x1b[200~'):
                pasted=True;pending=pending[6:];continue
            if pending.startswith(b'\x1b[201~'):
                pasted=False;pending=pending[6:];continue
            if pending.startswith(b'\x1b') and len(pending)<6: break
            byte,pending=pending[:1],pending[1:]
            if byte in (b'\n',b'\r') and not pasted:
                submits+=1
                if instruction.get('ack',True):
                    turn='synthetic-turn-'+str(submits);state={'state':'running'};sequence+=1
                (root/(config['session']+'.submits')).write_text(str(submits))
time.sleep(min(3,max(0,instruction.get("stop_delay",0))))
if not instruction.get("stop_ack",True): os._exit(9)
value.update(stopped=True,sequence=sequence+1,heartbeat_ms=time.time_ns()//1000000)
temporary=path.with_suffix('.tmp');temporary.write_text(json.dumps(value));temporary.replace(path)

#!/usr/bin/env python3
"""Disposable stdio MCP server: real elicitation protocol, no external services.

Protocol: https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation
Only returns a lab color; no credentials or account information are requested.
"""
import json
from pathlib import Path
import sys
import time


def main():
    log = Path(sys.argv[1]).open('a')
    pending = {}
    def record(direction, message):
        log.write(json.dumps(dict(at_ms=time.time_ns()//1_000_000, direction=direction, message=message))+'\n')
        log.flush()
    def send(message):
        message = dict(jsonrpc='2.0', **message)
        record('out', message)
        print(json.dumps(message), flush=True)
    for line in sys.stdin:
        message = json.loads(line)
        record('in', message)
        method, identity = message.get('method'), message.get('id')
        if method == 'initialize':
            send(dict(id=identity, result=dict(protocolVersion='2025-06-18', capabilities={'tools': {}},
                                              serverInfo={'name':'mesimon-state-lab', 'version':'1'})))
        elif method == 'tools/list':
            send(dict(id=identity, result={'tools':[{'name':'ask',
                'description':'Requests a harmless color choice through an interactive lab form.',
                'inputSchema':{'type':'object','properties':{},'additionalProperties':False}}]}))
        elif method == 'tools/call':
            call_id = 'lab-elicitation-'+str(identity)
            pending[call_id] = identity
            send(dict(id=call_id, method='elicitation/create', params={
                'message':'LAB_ELICIT: choose a lab color',
                'requestedSchema':{'type':'object','properties':{
                    'color':{'type':'string','title':'Color','default':'Blue'}},'required':['color']}}))
        elif method == 'ping':
            send(dict(id=identity, result={}))
        elif method is None and identity in pending:
            call = pending.pop(identity)
            send(dict(id=call,result={'content':[{'type':'text','text':json.dumps(message.get('result',message.get('error')))}]}))
        elif method in ('resources/list','prompts/list'):
            send(dict(id=identity, result={method.split('/')[0]:[]}))
        elif method and identity is not None:
            send(dict(id=identity,error={'code':-32601,'message':'Unknown lab method'}))
    log.close()


if __name__ == '__main__':
    main()

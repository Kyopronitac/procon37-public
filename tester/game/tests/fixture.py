"""Protocol fixture: no opponent strategy or private implementation."""
import json, os, pathlib, sys, time, subprocess
mode=sys.argv[1]
player=int(os.environ.get('PLAYER_ID','0'))
if mode=='cancel':
    child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'])
    pathlib.Path(f'started-{player}.json').write_text(json.dumps([os.getpid(),child.pid]))
    time.sleep(60)
for line in sys.stdin:
    req=json.loads(line)
    if mode=='exit':sys.exit(7)
    if mode=='timeout':time.sleep(3)
    if req['phase']=='select':
        assert req['problem']['players']==5
        n=len(req['problem']['agents'])
        if mode=='barrier':
            pathlib.Path(f'select-{player}').touch()
            until=time.monotonic()+1.5
            while len(list(pathlib.Path('.').glob('select-*')))<5:
                assert time.monotonic()<until, 'selection requests were serialized'
                time.sleep(.005)
        answer=[9] if mode=='invalid' else [0]*n
    else:
        others=req['state']['others']
        assert [o['id'] for o in others]==[i for i in range(5) if i!=player]
        assert all(len(o['agents'])==n for o in others)
        assert req['state']['day']==req['requestId']-1
        if mode=='invalid' and req['requestId']<=2:assert req['previousAccepted'] is False
        answer=[[-req['steps']] for _ in range(n)]
        if mode=='invalid' and req['requestId']==1:answer[0]=[6]
    print(json.dumps({'requestId':req['requestId'],'answer':answer}),flush=True)

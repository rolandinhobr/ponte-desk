"""Native coordinator integration test, isolated loopback + dummy input only."""
import hashlib,hmac,json,os,pathlib,signal,socket,subprocess,sys,tempfile,time
exe=pathlib.Path(sys.argv[1]).resolve()
root=pathlib.Path(tempfile.mkdtemp(prefix='ponte-desk-smoke-'))
key=b'K'*32
(root/'preferences.json').write_text(json.dumps(dict(shared_monitor=True,manage_displays=True,windows_monitor='test',mac_monitor_uuid='test',mac_input=19,pc_input=15)))
(root/'observer.json').write_text(json.dumps(dict(side='mac',key=key.hex(),peers=[],mac_input=19,pc_input=15,auto_display=False,port=14243,bind_address='127.0.0.1')))
(root/'config.toml').write_text('port = 14242\n')
env=dict(os.environ,PONTE_DESK_HOME=str(root),LAN_MOUSE_SOCKET_PATH=str(root/'ipc.sock'),LAN_MOUSE_IPC_PORT='15252')
for name in ['LAN_MOUSE_INTEGRATION_CONTROL']:env.pop(name,None)
sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);sock.bind(('127.0.0.1',0));sock.setblocking(False)
log=open(root/'test.log','wb')
p=subprocess.Popen([str(exe),'--capture-backend','dummy','--emulation-backend','dummy','daemon'],env=env,stdout=log,stderr=log)
seq=0;state=dict(monitor_present=True,fullscreen=False,input=15,input_age_ms=0,asleep=False,agent=False,manual_pause=False,capture=True,emulation=True,ack=0,direct_supported=True,direct_active=False)
state["preferences"]=None
def send():
    global seq
    seq+=1
    body=dict(v=4,side='pc',session='51'*16,seq=seq,time_ms=int(time.time()*1000),state=state)
    payload=json.dumps(body,separators=(',',':')).encode()
    packet=json.dumps(dict(body=body,tag=hmac.new(key,payload,hashlib.sha256).hexdigest()),separators=(',',':')).encode()
    sock.sendto(packet,('127.0.0.1',14243))
def wait(expected,seconds=4,send_peer=True):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        if send_peer:send()
        time.sleep(.15)
        if p.poll() is not None:raise AssertionError((root/'test.log').read_text())
        try:
            actual=json.loads((root/'engine-status.json').read_text())
            if (actual['capture_installed'],actual['emulation_enabled'])==expected:return actual
        except (FileNotFoundError,json.JSONDecodeError):pass
    raise AssertionError(f'{expected} not reached; '+(root/'engine-status.json').read_text())
try:
    first=wait((True,True));pid=first['pid']
    state['input']=19;wait((False,True))
    state['fullscreen']=True;wait((False,False))
    state['fullscreen']=False;state['input']=15;wait((True,True))
    # Hot options must not require a restart or DDC for ordinary sharing.
    options=json.loads((root/'preferences.json').read_text());options['shared_monitor']=False
    (root/'preferences.json').write_text(json.dumps(options));state['input']=None;state['input_age_ms']=None
    wait((True,True))
    # Fixed target keeps the native receiver available during local automation.
    options['input_mode']='to_mac';(root/'preferences.json').write_text(json.dumps(options))
    state.update(capture=True,emulation=False,direct_active=True,ack=int(time.time()*1000))
    wait((False,True))
    agent=subprocess.Popen([str(exe),'agent','begin','--owner','direct-regression','--ttl','30'],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    deadline=time.monotonic()+9
    while agent.poll() is None and time.monotonic()<deadline:
        send();time.sleep(.1)
    try:
        output,error=agent.communicate(timeout=1)
        assert agent.returncode==0,error
        token=output.strip();assert (root/'agent-leases'/(token+'.json')).exists()
        wait((False,True))
    finally:
        if agent.poll() is None:agent.kill()
        if agent.returncode==0:subprocess.run([str(exe),'agent','end',output.strip()],env=env,check=True)
    state['agent']=True;wait((False,False));state['agent']=False;wait((False,True))
    # Reverse direction waits for the old local receiver and peer capture to stop.
    options['input_mode']='to_pc';(root/'preferences.json').write_text(json.dumps(options))
    state.update(capture=False,emulation=True,direct_active=False,ack=0)
    wait((True,False))
    options['input_mode']='edges';(root/'preferences.json').write_text(json.dumps(options))
    state.update(capture=True,emulation=True)
    wait((True,True))
    (root/'preferences.json').write_text('{invalid');wait((False,False))
    (root/'preferences.json').write_text(json.dumps(options));wait((True,True))
    (root/'manual-pause').write_text('user');wait((False,False));(root/'manual-pause').unlink();wait((True,True))
    state['agent']=True;wait((False,False))
    state['agent']=False;wait((True,True))
    last=wait((False,False),seconds=8,send_peer=False)
    assert last['pid']==pid
    print(json.dumps(dict(passed=True,pid=pid,routing=True,fullscreen_suspend=True,agent_suspend=True,stale_peer_fails_closed=True,same_process=True,fixed_modes=True,target_agent_keeps_receiver=True,root=str(root))))
finally:
    if p.poll() is None:
        p.send_signal(signal.SIGINT)
        try:p.wait(4)
        except subprocess.TimeoutExpired:p.kill()
    log.close();sock.close()

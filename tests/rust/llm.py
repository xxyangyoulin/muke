#!/usr/bin/env python3
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from core import Client
from contract import ROOT, free_port, wait_ready


def main():
    calls=[]
    attempts={}
    gate=threading.Event()
    timeout_gate=threading.Event()
    class Handler(BaseHTTPRequestHandler):
        def log_message(self,*args): pass
        def do_POST(self):
            value=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            calls.append((self.path,dict(self.headers),value))
            model=value['model']
            attempts[model]=attempts.get(model,0)+1
            if model=='slow': time.sleep(8)
            if model=='timeout': timeout_gate.wait(150)
            if model=='gate': gate.wait(30)
            status = 0
            if model=='retry' and attempts[model] < 3: status=429 if attempts[model]==1 else 503
            if model in ['exhaust-429','exhaust-503','backoff']: status=503 if model=='exhaust-503' else 429
            if status:
                self.send_response(status);self.end_headers();return
            if model=='disconnect':
                self.connection.shutdown(socket.SHUT_RDWR);self.connection.close();return
            if model=='redirect':
                self.send_response(307);self.send_header('Location',f'http://localhost:{server.server_port}/v1/responses');self.end_headers();return
            if value['model']=='failed': self.send_response(401);self.end_headers();return
            if self.path.endswith('/responses'):
                body={'status':'completed','output':[{'type':'message','content':[{'type':'output_text','text':'analysis result'}]}],'usage':{'input_tokens':12,'output_tokens':3}}
            elif self.path.endswith('/chat/completions'):
                body={'choices':[{'finish_reason':'stop','message':{'content':'analysis result'}}],'usage':{'prompt_tokens':12,'completion_tokens':3}}
            else:
                body={'stop_reason':'end_turn','content':[{'type':'text','text':'analysis result'}],'usage':{'input_tokens':12,'output_tokens':3}}
            self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers()
            try:self.wfile.write(json.dumps(body).encode())
            except (BrokenPipeError,ConnectionResetError):pass
    server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
    threading.Thread(target=server.serve_forever,daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='muke-llm-') as temporary:
        root=Path(temporary)
        repo=root/'plugins';repo.mkdir()
        plugin=repo/'plugins/hello'
        shutil.copytree(ROOT/'examples/standalone-plugin',plugin,ignore=shutil.ignore_patterns('target','.git'))
        subprocess.run(['git','init','-q',str(repo)],check=True)
        manifest=json.loads((plugin/'plugin.json').read_text());manifest['capabilities']=['llm.generate'];(plugin/'plugin.json').write_text(json.dumps(manifest))
        source=(plugin/'src/main.rs').read_text()
        start=source.index('        Some("sync") => {');end=source.index('        Some("data")',start)
        source=source[:start]+'''        Some("sync" | "data") => {
            use std::io::Write;
            let profile=request["params"]["profile"].as_i64().or_else(||request["params"]["query"]["profile"].as_str().and_then(|value|value.parse::<i64>().ok()));
            println!("{}",json!({"type":"host_call","id":"one","method":"llm.generate","params":{"profile":profile,"instructions":"summarize","input":"changes"}}));
            io::stdout().flush()?;
            let mut reply=String::new();io::stdin().lock().read_line(&mut reply)?;
            let reply:Value=serde_json::from_str(&reply)?;
            if !reply["error"].is_null() { println!("{}",json!({"type":"error","message":reply["error"]}));return Ok(()); }
            json!({"result":reply["result"],"state":{"analysis":reply["result"]["text"]}})
        }
''' + source[end:]
        source=source.replace('        Some("data") => json!({"result":{"message":message}}),\n','')
        (plugin/'src/main.rs').write_text(source)
        other=repo/'plugins/other';other.mkdir()
        description=dict(manifest);description['id']='other';description['name']='Other plugin'
        (other/'plugin.json').write_text(json.dumps(description))
        base=f'http://127.0.0.1:{free_port()}'
        env={k:v for k,v in os.environ.items() if not k.startswith('MKBLOG_')};env['CARGO_NET_OFFLINE']='true';env.setdefault('CARGO_HOME',str(Path.home()/'.cargo'))
        process=subprocess.Popen([str(ROOT/'target/debug/muke'),'--data-dir',str(root/'data'),'serve','--listen',base[7:],'--frontend',str(ROOT/'frontend')],env=env,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        try:
            wait_ready(base,process)
            admin,guest=Client(base),Client(base);admin.state();guest.state()
            admin.call('setup',{'username':'tester','password':'Password12345'});admin.state()
            assert guest.call('llm-profiles')[0]==401
            assert guest.call('save-llm-profile',{})[0]==401
            def job(task):return next(j for j in admin.call('tasks')[1]['jobs'] if j['id']==task)
            def done(task):
                for _ in range(1600):
                    row=job(task)
                    if row['state'] not in ['queued','running']:return row
                    time.sleep(.1)
                raise AssertionError(job(task))
            status,result=admin.call('discover-plugins',{'source_kind':'local','source':str(repo)})
            assert status==200,result
            assert done(result['task_id'])['result']['plugins'][0]['path']=='plugins/hello'
            assert admin.call('install-plugin',{'source_kind':'local','source':str(repo),'path':'../escape'})[0]==422
            status,result=admin.call('install-plugin',{'source_kind':'local','source':str(repo),'path':'plugins/hello'})
            assert status==200,result
            assert done(result['task_id'])['state']=='succeeded',job(result['task_id'])
            assert admin.call('save-plugin',{'id':'hello','enabled':1,'values':{}})[0]==200
            def profile(protocol,model='test',id=None):
                values={'id':id,'name':protocol,'protocol':protocol,'base_url':f'http://127.0.0.1:{server.server_port}/v1','api_key':'secret-never-public','model':model,'max_tokens':1024}
                status,result=admin.call('save-llm-profile',values);assert status==200,result
                return result['id']
            def sync(id,plugin_id='hello'):return admin.call('sync-plugin',{'id':plugin_id,'params':{'profile':id}})[1]['task_id']
            def wait_for(predicate):
                for _ in range(200):
                    if predicate():return
                    time.sleep(.05)
                raise AssertionError('condition timed out')
            def call_record(task):return next(c for c in admin.call('llm-profiles')[1]['calls'] if c['task_id']==task)
            for protocol in ['responses','chat_completions','anthropic']:
                id=profile(protocol)
                outcome=done(sync(id));assert outcome['state']=='succeeded',outcome
                assert outcome['result']['plugin_result']['text']=='analysis result'
                data=admin.call('llm-profiles')[1]
                assert data['calls'][0]['input_tokens']==12
                assert 'secret-never-public' not in json.dumps(data)
                headers=calls[-1][1]
                assert ('x-api-key' in {k.lower() for k in headers})==(protocol=='anthropic')
            # 一键连通性测试：已保存配置、表单值（未保存）与失败路径
            assert guest.call('test-llm-profile',{'id':1})[0]==401
            tested=admin.call('test-llm-profile',{'id':profile('responses')})[1]
            assert tested['ok'] and isinstance(tested['ms'],int) and tested['text']=='analysis result',tested
            form_values={'id':None,'protocol':'responses','base_url':f'http://127.0.0.1:{server.server_port}/v1','model':'form-model','max_tokens':256,'api_key':'form-key'}
            assert admin.call('test-llm-profile',form_values)[1]['text']=='analysis result'
            status,error=admin.call('test-llm-profile',{**form_values,'protocol':'anthropic','model':'failed'})
            assert status==422 and 'HTTP 401' in error['error'],error
            status,error=admin.call('test-llm-profile',{**form_values,'api_key':''})
            assert status==422 and 'API Key' in error['error'],error
            status,error=admin.call('test-llm-profile',{'id':99999})
            assert status==404,error
            subprocess.run(['node',str(ROOT/'tests/rust/llm-browser.cjs'),base,str(repo)],check=True)
            assert 'secret-never-public' not in json.dumps(guest.state())
            assert done(sync(profile('responses','failed')))['state']=='failed'
            task=sync(profile('responses','slow'))
            for _ in range(100):
                if calls[-1][2]['model']=='slow':break
                time.sleep(.1)
            admin.call('cancel-task',{'id':task})
            assert done(task)['state']=='cancelled'
            assert admin.call('llm-profiles')[1]['calls'][0]['status']=='cancelled'
            # Page reads cannot initiate paid calls, even for a plugin that declares the capability.
            count=len(calls)
            status,error=admin.call('plugin-data',id='hello',profile=id)
            assert status==500 and error['error'],error
            assert len(calls)==count
            for name,capabilities in [('other',[]),('third',['llm.generate'])]:
                directory=repo/'plugins'/name
                shutil.copytree(plugin,directory,dirs_exist_ok=True)
                description=dict(manifest);description.update(id=name,name=name,capabilities=capabilities)
                description['page']={**manifest['page'],'route':'/'+name}
                (directory/'plugin.json').write_text(json.dumps(description))
                (directory/'src/main.rs').write_text(source.replace('"id":"hello"',f'"id":"{name}"'))
                (directory/'frontend/plugin.js').write_text((plugin/'frontend/plugin.js').read_text().replace("'/hello'",f"'/{name}'"))
                installed=admin.call('install-plugin',{'source_kind':'local','source':str(repo),'path':'plugins/'+name})[1]
                assert done(installed['task_id'])['state']=='succeeded'
                assert admin.call('save-plugin',{'id':name,'enabled':1,'values':{}})[0]==200
            count=len(calls)
            denied=done(sync(id,'other'))
            assert denied['state']=='failed' and '未声明' in denied['error'],denied
            assert len(calls)==count
            description=json.loads((other/'plugin.json').read_text());description['capabilities']=['llm.generate']
            (other/'plugin.json').write_text(json.dumps(description))
            installed=admin.call('install-plugin',{'source_kind':'local','source':str(repo),'path':'plugins/other'})[1]
            assert done(installed['task_id'])['state']=='succeeded'
            shared=profile('responses','gate')
            running=[sync(shared,name) for name in ['hello','other']]
            wait_for(lambda:attempts.get('gate',0)==2)
            waiting=sync(shared,'third');wait_for(lambda:job(waiting)['state']=='running')
            time.sleep(.5);assert attempts['gate']==2
            admin.call('cancel-task',{'id':waiting});assert done(waiting)['state']=='cancelled'
            assert attempts['gate']==2
            assert not any(c['task_id']==waiting for c in admin.call('llm-profiles')[1]['calls'])
            # 运行中的任务带日志，正好用于任务页的行内日志、过滤与弹窗验证
            subprocess.run(['node',str(ROOT/'tests/rust/tasks-browser.cjs'),base],check=True)
            gate.set()
            assert all(done(task)['state']=='succeeded' for task in running)
            assert done(sync(shared,'third'))['state']=='succeeded'
            print('PASS: capability rejection, page-read rejection, shared concurrency limit and cancellation while waiting',flush=True)
            # Exercise the actual 120-second request timeout while checking retries on another plugin.
            timeout_start=time.monotonic()
            timeout_task=sync(profile('responses','timeout'),'other')
            wait_for(lambda:attempts.get('timeout')==1)
            assert done(sync(profile('responses','retry')))['state']=='succeeded'
            assert attempts['retry']==3
            for model in ['exhaust-429','exhaust-503','disconnect','redirect']:
                task=sync(profile('responses',model));outcome=done(task)
                assert outcome['state']=='failed',outcome
                assert call_record(task)['status']=='failed'
                assert attempts[model]==(3 if model.startswith('exhaust-') else 1)
            backoff=sync(profile('responses','backoff'))
            wait_for(lambda:attempts.get('backoff')==1)
            admin.call('cancel-task',{'id':backoff});assert done(backoff)['state']=='cancelled'
            time.sleep(2.2)
            assert attempts['backoff']==1 and call_record(backoff)['status']=='cancelled'
            print('PASS: 429/503 retries and attempt limits, no retries after disconnect, credential redirect rejection and backoff cancellation',flush=True)
            outcome=done(timeout_task)
            assert outcome['state']=='failed' and '超时' in outcome['error'],outcome
            assert 115 <= time.monotonic()-timeout_start < 145
            assert attempts['timeout']==1 and call_record(timeout_task)['status']=='failed'
            timeout_gate.set()
            assert done(sync(profile('responses','after-timeout'),'other'))['state']=='succeeded'
            print('PASS: actual request timeout, failed-call recording and permit recovery',flush=True)
            print('PASS: model protocols, private credentials, usage, nested plugin discovery/install, host calls, failures and cancellation')
        finally:
            gate.set();timeout_gate.set()
            process.terminate();process.wait(timeout=15)
    server.shutdown();server.server_close()


if __name__=='__main__':main()

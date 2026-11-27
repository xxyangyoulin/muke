#!/usr/bin/env python3
import concurrent.futures
import hashlib
import http.server
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import threading
import time
import urllib.request
from core import Client
from contract import ROOT, free_port, wait_ready


def until(check, seconds=15):
    deadline=time.monotonic()+seconds
    while time.monotonic()<deadline:
        value=check()
        if value:return value
        time.sleep(.05)
    raise AssertionError('condition timed out')


def main():
    with tempfile.TemporaryDirectory(prefix='muke-plugins-') as temporary:
        root=Path(temporary);data=root/'data'
        picture=root/'picture.png'
        subprocess.run(['magick','-size','900x600','plasma:fractal',str(picture)],check=True)
        image=picture.read_bytes()
        lock=threading.Lock()
        state={'targets':[],'requests':[],'slow':False,'active':0,'peak':0,'referers':[]}
        class Proxy(http.server.BaseHTTPRequestHandler):
            protocol_version='HTTP/1.1'
            def log_message(self,*args):pass
            def do_CONNECT(self):
                if self.path.endswith(':443'):
                    self.send_error(502);return
                with lock:state['targets'].append(self.path)
                self.send_response(200);self.end_headers();self.close_connection=False
            def do_GET(self):
                with lock:
                    state['requests'].append(self.path);state['referers'].append((self.path,self.headers.get('Referer')));state['active']+=1;state['peak']=max(state['peak'],state['active'])
                try:
                    if state['slow']:time.sleep(3)
                    else:time.sleep(.1)
                    status=200;headers={}
                    if self.path in ['/picture','/weibo-picture','/weibo-final']:body=image
                    elif self.path=='/weibo-cross':
                        status=302;headers['Location']='http://1.1.1.1/weibo-final';body=b''
                    elif self.path=='/weibo-same':
                        status=302;headers['Location']='/weibo-picture';body=b''
                    elif self.path.startswith('/weibo-feed'):
                        body=b'''<feed xmlns="http://www.w3.org/2005/Atom"><title>Weibo</title>
<entry><link rel="alternate" href="https://weibo.com/status/123"/><author><name>Fixture author</name></author><summary><![CDATA[<p>Weibo &amp; fixture</p><img src="http://8.8.8.8/weibo-same"><img src="http://8.8.8.8/weibo-cross"><img src="http://127.0.0.1/private-image"><img src="http://8.8.8.8/broken-image"><img src="http://8.8.8.8/fifth-image">]]></summary><updated>2026-11-26T12:00:00Z</updated></entry>
<entry><link href="https://weibo.com/status/456"/><title>Second weibo</title></entry></feed>'''
                    elif self.path=='/redirect-image':
                        status=302;headers['Location']='http://127.0.0.1/private-image';body=b''
                    elif self.path=='/private-image':raise AssertionError('SSRF reached fixture')
                    elif self.path.startswith('/broken'):status=500;body=b'failed'
                    else:
                        body=f'''<rss version="2.0"><channel><title>Fixture {self.path}</title>
<item><guid>{self.path}-new</guid><title>Current entry</title><description><![CDATA[<p>Summary &amp; text</p><img src="http://8.8.8.8/picture"><img src="http://127.0.0.1/private-image"><img src="http://8.8.8.8/redirect-image">]]></description></item>
<item><guid>{self.path}-old</guid><title>Old entry</title><pubDate>2000-01-01 08:00:00</pubDate></item>
</channel></rss>'''.encode()
                    self.send_response(status)
                    for key,value in headers.items():self.send_header(key,value)
                    self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
                except (BrokenPipeError,ConnectionResetError):pass
                finally:
                    with lock:state['active']-=1
        proxy=http.server.ThreadingHTTPServer(('127.0.0.1',0),Proxy)
        threading.Thread(target=proxy.serve_forever,daemon=True).start()
        config=root/'config.toml';config.write_text(f'http_proxy = "http://127.0.0.1:{proxy.server_port}"\n')
        base=f'http://127.0.0.1:{free_port()}'
        command=[str(ROOT/'target/debug/muke'),'--config',str(config),'--data-dir',str(data),'serve','--listen',base[7:],'--frontend',str(ROOT/'frontend')]
        log=(root/'server.log').open('w+')
        process=subprocess.Popen(command,stdout=log,stderr=log)
        try:
            wait_ready(base,process)
            admin=Client(base);guest=Client(base);admin.state();guest.state()
            assert admin.call('setup',{'username':'tester','password':'Password12345'})[0]==200
            admin.state()
            for action in ['plugins','tasks']:
                assert guest.call(action)[0]==401
            assert guest.call('cancel-task',{'id':1})[0]==401
            try:guest.http.open(base+'/api/tasks/events');raise AssertionError('SSE must require admin')
            except urllib.error.HTTPError as error:assert error.code==401
            for path in ['/plugins/rss/backend/mod.rs','/plugins/rss/manifest.json']:
                try:guest.http.open(base+path);raise AssertionError('backend plugin files must not be served')
                except urllib.error.HTTPError as error:assert error.code==404
            assert guest.http.open(base+'/plugins/rss/plugin.js').status==200
            plugin=admin.call('plugins')[1]['plugins'][0]
            fields={field['key']:field['value'] for field in plugin['fields']}
            assert fields['auto_delete']=='1' and fields['retention_days']=='30'
            assert admin.call('save-plugin',{'id':'rss','enabled':1,'values':{'retention_days':0}})[0]==422
            assert not admin.call('plugins')[1]['plugins'][0]['enabled']
            assert admin.call('save-plugin',{'id':'rss','enabled':1,'values':{}})[0]==200
            def jobs():return admin.call('tasks')[1]['jobs']
            def job(id):return next(job for job in jobs() if job['id']==id)
            def done(id):return until(lambda:job(id) if job(id)['state'] not in ['queued','running'] else None)
            status,result=admin.call('plugin-admin',{'id':'rss','op':'add','url':'http://127.0.0.1/feed'})
            assert status==200,result
            fid=result['id'];task=result['task_id'];assert done(task)['state']=='succeeded',job(task)
            rows=guest.call('plugin-data',id='rss')[1]['data'];assert rows['total']==1,rows
            post=rows['posts'][0];assert post['source_key']=='rss:'+fid+':'+hashlib.sha1(b'/feed-new').hexdigest()[:16]
            assert 'url' not in rows['feeds'][0] and 'last_error' not in rows['feeds'][0]
            assert guest.call('posts')[1]['total']==0
            assert len(post['images'])==1,post
            image_name=post['images'][0]['ref'][3:]
            original=data/'uploads'/image_name;thumb=data/'rss-thumbs'/(image_name+'.webp')
            assert 0<original.stat().st_size<100*1024
            assert 0<thumb.stat().st_size<50*1024
            assert guest.http.open(base+post['images'][0]['thumb']).read()==thumb.read_bytes()
            assert '8.8.8.8:80' in state['targets']
            assert '/private-image' not in state['requests']
            task=admin.call('sync-plugin',{'id':'rss'})[1]['task_id']
            assert done(task)['result']['added']==0
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute("UPDATE posts SET created_at='2000-01-01T00:00:00Z' WHERE id=?",(post['id'],))
                db.execute("INSERT INTO post_likes VALUES(?,?,'2000')",(post['id'],'test'))
                db.execute("INSERT INTO comments(kind,target_id,author,content,created_at) VALUES('post',?,'a','c','2000')",(post['id'],))
            task=admin.call('sync-plugin',{'id':'rss'})[1]['task_id'];assert done(task)['state']=='succeeded'
            assert not original.exists() and not thumb.exists()
            with sqlite3.connect(data/'blog.sqlite') as db:
                assert db.execute('SELECT COUNT(*) FROM post_likes').fetchone()[0]==0
                assert db.execute('SELECT COUNT(*) FROM comments').fetchone()[0]==0
            post=guest.call('plugin-data',id='rss')[1]['data']['posts'][0]
            image_name=post['images'][0]['ref'][3:]
            assert admin.call('delete-post',{'id':post['id']})[0]==200
            assert not (data/'uploads'/image_name).exists() and not (data/'rss-thumbs'/(image_name+'.webp')).exists()
            task=admin.call('sync-plugin',{'id':'rss'})[1]['task_id'];assert done(task)['result']['added']==0
            # Persistent schedule and independent feed requests, with two compressor jobs active.
            with sqlite3.connect(data/'blog.sqlite') as db:
                for i in range(4):
                    url=f'http://127.0.0.1/feed-{i}';id=hashlib.sha1(url.encode()).hexdigest()[:12]
                    db.execute('INSERT INTO rss_feeds(id,url,created_at) VALUES(?,?,?)',(id,url,'2026'))
            state['slow']=True
            task=admin.call('sync-plugin',{'id':'rss'})[1]['task_id']
            assert admin.call('sync-plugin',{'id':'rss'})[1]['task_id']==task
            until(lambda:job(task)['state']=='running')
            until(lambda:state['active']>=3)
            assert admin.call('purge-plugin-posts',{'id':'rss'})[0]==409
            assert admin.call('remove-plugin',{'id':'rss','purge':1})[0]==409
            start=time.monotonic()
            with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
                responses=list(executor.map(lambda _:guest.call('posts')[0],range(40)))
            assert responses==[200]*40 and time.monotonic()-start<4
            start=time.monotonic();assert admin.call('cancel-task',{'id':task})[1]['cancelled']
            assert done(task)['state']=='cancelled';assert time.monotonic()-start<4
            assert not (data/'runtime'/f'job-{task}').exists()
            state['slow']=False
            task=admin.call('sync-plugin',{'id':'rss'})[1]['task_id'];assert done(task)['state']=='succeeded'
            assert guest.call('plugin-data',id='rss')[1]['data']['total']==4
            until(lambda:(data/'runtime'/'tasks.sqlite').exists())
            until(lambda:sqlite3.connect(data/'runtime'/'tasks.sqlite').execute('SELECT COUNT(*) FROM schedules').fetchone()[0]==1)
            with sqlite3.connect(data/'runtime'/'tasks.sqlite') as db:db.execute('UPDATE schedules SET next_run=0')
            latest=max(j['id'] for j in jobs())
            scheduled=until(lambda:next((j for j in jobs() if j['id']>latest),None),8)
            assert done(scheduled['id'])['state']=='succeeded'
            subprocess.run(['node',str(ROOT/'tests/rust/plugins-browser.cjs'),base],check=True)
            first=guest.call('plugin-data',id='rss')[1]['data']['posts'][0]
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute("CREATE TRIGGER fail_remove BEFORE DELETE ON posts BEGIN SELECT RAISE(ABORT,'fixture failure'); END")
            assert admin.call('plugin-admin',{'id':'rss','op':'remove','feed_id':first['meta']['feed'],'purge':1})[0]==500
            with sqlite3.connect(data/'blog.sqlite') as db:
                assert db.execute('SELECT COUNT(*) FROM rss_feeds WHERE id=?',(first['meta']['feed'],)).fetchone()[0]==1
                assert db.execute('SELECT COUNT(*) FROM posts WHERE id=?',(first['id'],)).fetchone()[0]==1
                db.execute('DROP TRIGGER fail_remove')
            # Shared avatar references survive individual deletions; purge cleans plugin-owned files.
            posts=guest.call('plugin-data',id='rss')[1]['data']['posts']
            avatar_name='f'*32+'.webp'
            avatar=data/'uploads'/avatar_name
            avatar_thumb=data/'rss-thumbs'/(avatar_name+'.webp')
            avatar.write_bytes((data/'uploads'/posts[0]['images'][0]['ref'][3:]).read_bytes())
            avatar_thumb.write_bytes((data/'rss-thumbs'/(posts[0]['images'][0]['ref'][3:]+'.webp')).read_bytes())
            with sqlite3.connect(data/'blog.sqlite') as db:
                for post in posts[:2]:
                    meta=post['meta'];meta['avatar']='up:'+avatar_name
                    db.execute('UPDATE posts SET meta=? WHERE id=?',(json.dumps(meta),post['id']))
            assert admin.call('delete-post',{'id':posts[0]['id']})[0]==200
            assert avatar.exists() and avatar_thumb.exists()
            assert admin.call('delete-post',{'id':posts[1]['id']})[0]==200
            assert not avatar.exists() and not avatar_thumb.exists()
            assert admin.call('save-plugin',{'id':'rss','enabled':0})[0]==200
            remaining=guest.call('posts')[1]['posts'];assert len(remaining)==2
            saved_url=remaining[0]['images'][0]['thumb']
            assert guest.http.open(base+saved_url).status==200
            assert guest.call('plugin-data',id='rss')[0]==404
            assert admin.call('save-plugin',{'id':'rss','enabled':1})[0]==200
            assert guest.call('purge-plugin-posts',{'id':'rss'})[0]==401
            status,purge=admin.call('purge-plugin-posts',{'id':'rss'})
            assert status==200 and purge['deleted']==2,purge
            assert not list((data/'uploads').iterdir())
            assert not list((data/'rss-thumbs').iterdir())
            with sqlite3.connect(data/'blog.sqlite') as db:
                assert db.execute('SELECT COUNT(*) FROM plugin_tombstones').fetchone()[0]==0
                assert db.execute('SELECT COUNT(*) FROM rss_feeds').fetchone()[0]==5
            task=admin.call('sync-plugin',{'id':'rss'})[1]['task_id'];assert done(task)['result']['added']==5
            assert admin.call('remove-plugin',{'id':'rss','purge':0})[0]==200
            assert guest.call('posts')[1]['total']==5
            assert admin.call('plugins')[1]['plugins'][0]['removed']
            assert admin.call('save-plugin',{'id':'rss','enabled':1})[0]==200
            assert admin.call('remove-plugin',{'id':'rss','purge':1})[0]==200
            assert guest.call('posts')[1]['total']==0
            assert not list((data/'uploads').iterdir())
            assert not list((data/'rss-thumbs').iterdir())
            # DMS establishes a baseline, writes events and snapshot atomically, and preserves it on errors.
            desktop=root/'desktop';(desktop/'plugins'/'music').mkdir(parents=True)
            manifest=desktop/'plugins'/'music'/'plugin.json'
            manifest.write_text(json.dumps({'id':'music','name':'音乐','version':'1'}))
            assert admin.call('save-plugin',{'id':'dms','enabled':1,'values':{}})[0]==422
            assert admin.call('save-plugin',{'id':'dms','enabled':1,'values':{'config_dir':'relative'}})[0]==422
            assert admin.call('save-plugin',{'id':'dms','enabled':1,'values':{'config_dir':str(desktop)}})[0]==200
            assert 'module' not in guest.call('state')[1]['plugins']['dms']
            task=admin.call('sync-plugin',{'id':'dms'})[1]['task_id'];assert done(task)['result']['added']==0
            def snapshot():
                with sqlite3.connect(data/'blog.sqlite') as db:return db.execute("SELECT value FROM settings WHERE key='plugin:dms:snapshot'").fetchone()[0]
            baseline=snapshot()
            manifest.write_text('{}')
            task=admin.call('sync-plugin',{'id':'dms'})[1]['task_id'];assert done(task)['state']=='failed'
            assert snapshot()==baseline
            manifest.write_text(json.dumps({'id':'music','name':'音乐','version':'2'}))
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute("CREATE TRIGGER fail_dms BEFORE INSERT ON posts WHEN NEW.source='dms' BEGIN SELECT RAISE(ABORT,'fixture failure'); END")
            task=admin.call('sync-plugin',{'id':'dms'})[1]['task_id'];assert done(task)['state']=='failed'
            assert snapshot()==baseline
            assert guest.call('posts')[1]['total']==0
            with sqlite3.connect(data/'blog.sqlite') as db:db.execute('DROP TRIGGER fail_dms')
            task=admin.call('sync-plugin',{'id':'dms'})[1]['task_id'];assert done(task)['result']['added']==1
            post=guest.call('posts')[1]['posts'][0]
            assert post['source']=='dms' and '音乐（1 → 2）' in post['content']
            seed=json.dumps([str(desktop.resolve()),1,'updated',post['content']],ensure_ascii=True,separators=(',',':')).replace('/',r'\/')
            assert post['source_key']=='snapshot:'+hashlib.sha256(seed.encode()).hexdigest()
            assert json.loads(snapshot())['revision']==2
            task=admin.call('sync-plugin',{'id':'dms'})[1]['task_id'];assert done(task)['result']['added']==0
            assert admin.call('delete-post',{'id':post['id']})[0]==200
            task=admin.call('sync-plugin',{'id':'dms'})[1]['task_id'];assert done(task)['result']['added']==0
            # Weibo keeps source keys, raw originals, Referer and immediate config updates.
            assert admin.call('save-plugin',{'id':'weibo','enabled':1,'values':{}})[0]==422
            assert admin.call('save-plugin',{'id':'weibo','enabled':1,'values':{'feed_url':'http://127.0.0.1/weibo-feed','max_items':1}})[0]==200
            assert guest.http.open(base+'/plugins/weibo/plugin.js').status==200
            task=admin.call('sync-plugin',{'id':'weibo'})[1]['task_id'];assert done(task)['result']['added']==1,job(task)
            post=guest.call('posts')[1]['posts'][0]
            assert post['source_key']=='weibo:123' and post['content']=='Weibo & fixture'
            assert post['meta']['name']=='Fixture author' and post['meta']['hide_source']==0
            assert len(post['images'])==2,post
            for item in post['images']:assert (data/'uploads'/item['ref'][3:]).read_bytes()==image
            assert ('/weibo-same','https://weibo.com/') in state['referers']
            assert ('/weibo-picture','https://weibo.com/') in state['referers']
            assert ('/weibo-cross','https://weibo.com/') in state['referers']
            assert ('/weibo-final',None) in state['referers']
            assert '/fifth-image' not in state['requests'] and '/private-image' not in state['requests']
            before=state['requests'].count('/weibo-picture')
            task=admin.call('sync-plugin',{'id':'weibo'})[1]['task_id'];assert done(task)['result']['added']==0
            assert state['requests'].count('/weibo-picture')==before
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute("CREATE TRIGGER fail_weibo_config BEFORE UPDATE ON posts WHEN OLD.source='weibo' BEGIN SELECT RAISE(ABORT,'fixture failure'); END")
            assert admin.call('save-plugin',{'id':'weibo','values':{'show_source':'0'}})[0]==500
            with sqlite3.connect(data/'blog.sqlite') as db:
                assert db.execute("SELECT value FROM settings WHERE key='plugin:weibo:show_source'").fetchone()[0]=='1'
                db.execute('DROP TRIGGER fail_weibo_config')
            assert admin.call('save-plugin',{'id':'weibo','values':{'show_source':'0'}})[0]==200
            assert guest.call('posts')[1]['posts'][0]['meta']['hide_source']==1
            task=admin.call('sync-plugin',{'id':'weibo','all':True})[1]['task_id'];assert done(task)['result']['added']==1
            assert guest.call('posts')[1]['total']==2
            subprocess.run(['node','tests/rust/weibo-browser.cjs',base],cwd=ROOT,check=True)
            assert admin.call('delete-post',{'id':post['id']})[0]==200
            assert not list((data/'uploads').iterdir())
            task=admin.call('sync-plugin',{'id':'weibo','all':True})[1]['task_id'];assert done(task)['result']['added']==0
            assert admin.call('purge-plugin-posts',{'id':'weibo'})[0]==200
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute("CREATE TRIGGER fail_weibo BEFORE INSERT ON posts WHEN NEW.source='weibo' BEGIN SELECT RAISE(ABORT,'fixture failure'); END")
            task=admin.call('sync-plugin',{'id':'weibo'})[1]['task_id'];assert done(task)['state']=='failed'
            assert not list((data/'uploads').iterdir())
            with sqlite3.connect(data/'blog.sqlite') as db:
                assert db.execute('SELECT COUNT(*) FROM plugin_pending_media').fetchone()[0]==0
                db.execute('DROP TRIGGER fail_weibo')
            state['slow']=True
            task=admin.call('sync-plugin',{'id':'weibo'})[1]['task_id']
            until(lambda:job(task)['state']=='running')
            assert admin.call('save-plugin',{'id':'weibo','enabled':0})[0]==409
            assert admin.call('cancel-task',{'id':task})[0]==200
            assert done(task)['state']=='cancelled'
            assert not (data/'runtime'/f'job-{task}').exists()
            state['slow']=False
            task=admin.call('sync-plugin',{'id':'weibo'})[1]['task_id'];assert done(task)['result']['added']==1
            post=guest.call('posts')[1]['posts'][0]
            assert admin.call('remove-plugin',{'id':'weibo','purge':0})[0]==200
            assert guest.http.open(base+post['images'][0]['url']).read()==image
            # Simulate interruption after media copy; recovery preserves committed references.
            process.terminate();process.wait(timeout=15)
            orphan_name='e'*32+'.png';orphan=data/'uploads'/orphan_name;orphan.write_bytes(image)
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute('INSERT INTO plugin_pending_media VALUES(?)',('up:'+orphan_name,))
                db.execute('INSERT INTO plugin_pending_media VALUES(?)',(post['images'][0]['ref'],))
            process=subprocess.Popen(command,stdout=log,stderr=log);wait_ready(base,process)
            assert not orphan.exists()
            assert guest.http.open(base+post['images'][0]['url']).read()==image
            with sqlite3.connect(data/'blog.sqlite') as db:assert db.execute('SELECT COUNT(*) FROM plugin_pending_media').fetchone()[0]==0
            assert admin.call('remove-plugin',{'id':'weibo','purge':1})[0]==200
            assert not list((data/'uploads').iterdir())
            # Douban config validation and imported historical movie cards, without contacting Douban.
            assert admin.call('save-plugin',{'id':'douban','enabled':1,'values':{}})[0]==422
            assert admin.call('save-plugin',{'id':'douban','values':{'user_id':'../../private'}})[0]==422
            assert admin.call('save-plugin',{'id':'douban','values':{'category':'other'}})[0]==422
            assert admin.call('save-plugin',{'id':'douban','values':{'history_limit':501}})[0]==422
            assert admin.call('save-plugin',{'id':'douban','values':{'user_id':'tester123','category':'wish','history_limit':35}})[0]==200
            assert guest.http.open(base+'/plugins/douban/plugin.js').status==200
            for path in ['/plugins/douban/backend/mod.rs','/plugins/douban/manifest.json']:
                try:guest.http.open(base+path);raise AssertionError('backend plugin files must not be served')
                except urllib.error.HTTPError as error:assert error.code==404
            cover_name='d'*32+'.png';(data/'uploads'/cover_name).write_bytes(image)
            meta={'card':'movie','title':'电影 <script>fixture</script>','year':'2020','intro':'2020-01-01 / 中国 / 导演','rating':'★★★★☆','comment':'很好 & 好看','date':'2026-11-26','category':'想看'}
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute("INSERT INTO posts(content,created_at,updated_at,images,meta,source,source_key,link) VALUES(?,?,?,?,?,'douban','douban:wish:123','https://movie.douban.com/subject/123/')",('想看电影','2026-11-26T21:00:00+08:00','2026-11-26T21:00:00+08:00',json.dumps(['up:'+cover_name]),json.dumps(meta)))
            subprocess.run(['node','tests/rust/douban-browser.cjs',base],cwd=ROOT,check=True)
            movie=guest.call('posts')[1]['posts'][0]
            assert movie['source_key']=='douban:wish:123' and movie['meta']==meta
            assert admin.call('remove-plugin',{'id':'douban','purge':0})[0]==200
            assert guest.http.open(base+movie['images'][0]['url']).read()==image
            assert admin.call('remove-plugin',{'id':'douban','purge':1})[0]==200
            assert not (data/'uploads'/cover_name).exists()
            # SSE snapshots reflect durable jobs and stop on logout (no stale admin stream).
            stream=admin.http.open(base+'/api/tasks/events',timeout=5)
            event=stream.readline();assert event==b'event: tasks\n',event
            payload=stream.readline();assert b'"jobs"' in payload
            assert admin.call('logout',{})[0]==200
            until(lambda:stream.readline()==b'',3)
            stream.close()
            print('PASS: plugin registry, RSS/Atom task pipeline, proxy IP pinning, SSRF redirect isolation, image limits, expiry media cleanup, tombstones, concurrent fetching, API responsiveness, cancellation, persistent schedules, DMS baseline/rollback, Weibo sync/config rollback/media recovery, Douban cards/config and authenticated SSE')
        except BaseException:
            log.flush();log.seek(0);print(log.read());raise
        finally:
            process.terminate();process.wait(timeout=15);proxy.shutdown();log.close()

if __name__=='__main__':main()

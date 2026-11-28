#!/usr/bin/env python3
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import urllib.error
import urllib.request
from core import Client
from contract import ROOT, free_port, wait_ready
from plugins import until


def main():
    with tempfile.TemporaryDirectory(prefix='muke-wallpapers-') as temporary:
        root=Path(temporary);data=root/'data';walls=root/'walls';walls.mkdir()
        original=walls/'风景 & 海.png'
        subprocess.run(['magick','-size','900x600','gradient:blue-green',str(original)],check=True)
        image=original.read_bytes()
        hidden=walls/'.hidden.png';hidden.write_bytes(image)
        (walls/'outside.png').symlink_to(original)
        outside=root/'private.txt';outside.write_text('private')
        (walls/'private.png').symlink_to(outside)
        base=f'http://127.0.0.1:{free_port()}'
        command=[str(ROOT/'target/debug/muke'),'--data-dir',str(data),'serve','--listen',base[7:],'--frontend',str(ROOT/'frontend')]
        log=(root/'server.log').open('w+')
        process=subprocess.Popen(command,stdout=log,stderr=log)
        try:
            wait_ready(base,process)
            admin=Client(base);guest=Client(base);admin.state();guest.state()
            assert admin.call('setup',{'username':'tester','password':'Password12345'})[0]==200
            admin.state()
            assert guest.call('plugin-data',id='wallpapers')[0]==404
            assert admin.call('save-plugin',{'id':'wallpapers','enabled':1,'values':{'wallpaper_dir':str(walls),'wallpaper_sync_interval':0,'wallpaper_post_template':'收了一张新壁纸：{name}'}})[0]==200
            def gallery():return guest.call('plugin-data',id='wallpapers')[1]['data']
            def job(id):return next(job for job in admin.call('tasks')[1]['jobs'] if job['id']==id)
            def done(id):return until(lambda:job(id) if job(id)['state'] not in ['queued','running'] else None)
            def sync():
                status,result=admin.call('sync-plugin',{'id':'wallpapers'});assert status==200,result
                return done(result['task_id'])
            result=sync();assert result['state']=='succeeded' and result['result']['scanned']==1,result
            assert guest.call('posts')[1]['total']==0
            assert gallery()['available'] and gallery()['items'][0]['id']==hashlib.sha256(original.name.encode()).hexdigest()
            assert guest.call('state')[1]['plugins']['wallpapers']['full_images']
            with sqlite3.connect(data/'blog.sqlite') as db:
                assert db.execute("SELECT value FROM settings WHERE key='wallpaper_dir'").fetchone()[0]==str(walls)
            nested=walls/'Portrait';nested.mkdir();second=nested/'竖屏.png'
            subprocess.run(['magick','-size','400x800','gradient:red-blue',str(second)],check=True)
            result=sync();assert result['result']['added']==1,result
            post=guest.call('posts')[1]['posts'][0]
            assert post['source_key']=='wall:'+hashlib.sha256('Portrait/竖屏.png'.encode()).hexdigest()
            assert post['content']=='收了一张新壁纸：竖屏.png' and len(post['images'])==1,post
            descriptor=post['images'][0]
            assert descriptor['width']==400 and descriptor['height']==800
            assert guest.http.open(base+descriptor['url']).read()==second.read_bytes()
            thumb=guest.http.open(base+descriptor['thumb']);assert thumb.headers['Content-Type']=='image/webp'
            assert thumb.read()[:4]==b'RIFF'
            response=guest.http.open(base+descriptor['url']+'&download=1');assert response.headers['Content-Disposition'].startswith('attachment;')
            for path in ['/image?file=../private.txt','/image?file=private.png','/image?file=/etc/passwd','/plugins/wallpapers/backend/mod.rs']:
                try:guest.http.open(base+path);raise AssertionError('private file was served')
                except urllib.error.HTTPError as error:assert error.code==404,error
            subprocess.run(['node','tests/rust/wallpapers-browser.cjs',base],cwd=ROOT,check=True)
            until(lambda:all(j['state'] not in ['queued','running'] for j in admin.call('tasks')[1]['jobs']))
            # An unavailable source must not erase the last successful gallery or posts.
            offline=root/'offline';walls.rename(offline)
            result=sync();assert result['state']=='failed',result
            assert not gallery()['available'] and len(gallery()['items'])==2
            assert guest.call('posts')[1]['total']==1
            offline.rename(walls)
            third=walls/'new.html';third.write_bytes(image)
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute("CREATE TRIGGER fail_wall BEFORE INSERT ON posts WHEN NEW.source='wallpapers' BEGIN SELECT RAISE(ABORT,'fixture failure'); END")
            result=sync();assert result['state']=='failed',result
            assert len(gallery()['items'])==2 and guest.call('posts')[1]['total']==1
            with sqlite3.connect(data/'blog.sqlite') as db:db.execute('DROP TRIGGER fail_wall')
            assert sync()['result']['added']==1
            # Partial removal from a multi-image post preserves the other image and discussion.
            first_id=hashlib.sha256(original.name.encode()).hexdigest()
            second_id=hashlib.sha256('Portrait/竖屏.png'.encode()).hexdigest()
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute('UPDATE posts SET images=? WHERE id=?',(json.dumps(['wall:'+first_id,'wall:'+second_id]),post['id']))
                db.execute("INSERT INTO comments(kind,target_id,author,content,created_at) VALUES('post',?,'A','C','2026')",(post['id'],))
                db.execute("INSERT INTO post_likes VALUES(?,'token','2026')",(post['id'],))
            second.unlink();assert sync()['state']=='succeeded'
            with sqlite3.connect(data/'blog.sqlite') as db:
                assert json.loads(db.execute('SELECT images FROM posts WHERE id=?',(post['id'],)).fetchone()[0])==['wall:'+first_id]
                assert db.execute('SELECT COUNT(*) FROM comments').fetchone()[0]==1
            original.unlink();assert sync()['result']['removed']==1
            with sqlite3.connect(data/'blog.sqlite') as db:
                assert db.execute('SELECT COUNT(*) FROM comments').fetchone()[0]==0
                assert db.execute('SELECT COUNT(*) FROM post_likes').fetchone()[0]==0
            original.write_bytes(image);assert sync()['result']['added']==1
            returned=next(p for p in guest.call('posts')[1]['posts'] if p['source_key']=='wall:'+first_id)
            assert admin.call('delete-post',{'id':returned['id']})[0]==200
            assert original.exists() and sync()['result']['added']==0
            saved=guest.call('posts')[1]['posts'][0]['images'][0]['url']
            assert admin.call('save-plugin',{'id':'wallpapers','enabled':0})[0]==200
            assert guest.call('plugin-data',id='wallpapers')[0]==404
            assert 'page' not in guest.call('state')[1]['plugins']['wallpapers']
            response=guest.http.open(base+saved);assert response.headers['Content-Type']=='image/png' and response.read()==image
            assert admin.call('save-plugin',{'id':'wallpapers','enabled':1})[0]==200
            # Cancellation during a scan leaves the committed gallery intact.
            for index in range(80):shutil.copyfile(third,walls/f'pending-{index}.png')
            task=admin.call('sync-plugin',{'id':'wallpapers'})[1]['task_id']
            until(lambda:job(task)['state']=='running')
            # 保存配置不再与插件任务互斥，执行中也可保存（立即成功）
            assert admin.call('save-plugin',{'id':'wallpapers','values':{}})[0]==200
            assert admin.call('cancel-task',{'id':task})[0]==200
            assert done(task)['state']=='cancelled'
            assert len(gallery()['items'])==2
            assert not (data/'runtime'/f'job-{task}').exists()
            for path in walls.glob('pending-*.png'):path.unlink()
            # The schedule uses seconds and keeps the legacy setting keys.
            assert admin.call('save-plugin',{'id':'wallpapers','values':{'wallpaper_sync_interval':17}})[0]==200
            def schedule():
                with sqlite3.connect(data/'runtime'/'tasks.sqlite') as db:
                    return db.execute("SELECT interval_seconds FROM schedules WHERE key='plugin:wallpapers'").fetchone()
            assert until(schedule,8)==(17,)
            assert admin.call('remove-plugin',{'id':'wallpapers','purge':0})[0]==200
            response=guest.http.open(base+saved);assert response.headers['Content-Type']=='image/png' and response.read()==image
            assert not next(p for p in admin.call('plugins')[1]['plugins'] if p['id']=='wallpapers')['enabled']
            # Deletion rollback and persistent recovery before/after the database commit.
            assert admin.call('save-plugin',{'id':'wallpapers','enabled':1,'values':{'wallpaper_sync_interval':0}})[0]==200
            third_id=hashlib.sha256(third.name.encode()).hexdigest()
            assert guest.call('plugin-admin',{'id':'wallpapers','op':'delete','wall_id':third_id})[0]==401
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute("CREATE TRIGGER fail_delete BEFORE DELETE ON wallpapers BEGIN SELECT RAISE(ABORT,'fixture failure'); END")
            assert admin.call('plugin-admin',{'id':'wallpapers','op':'delete','wall_id':third_id})[0]==500
            assert third.read_bytes()==image and guest.http.open(base+saved).read()==image
            with sqlite3.connect(data/'blog.sqlite') as db:
                assert db.execute('SELECT COUNT(*) FROM wallpaper_deletions').fetchone()[0]==0
                db.execute('DROP TRIGGER fail_delete')
            def stage_deletion():
                metadata=third.stat();rootmeta=walls.stat();staged=walls/'.deleted-fixture'
                with sqlite3.connect(data/'blog.sqlite') as db:
                    db.execute('INSERT INTO wallpaper_deletions VALUES(?,?,?,?,?,?,?,?)',(third_id,str(walls),third.name,staged.name,rootmeta.st_dev,rootmeta.st_ino,metadata.st_dev,metadata.st_ino))
                third.rename(staged)
                return staged
            process.terminate();process.wait(timeout=15)
            staged=stage_deletion()
            process=subprocess.Popen(command,stdout=log,stderr=log);wait_ready(base,process)
            assert third.read_bytes()==image and not staged.exists()
            # Offline media defers recovery without dropping the journal or gallery.
            process.terminate();process.wait(timeout=15)
            staged=stage_deletion();walls.rename(offline)
            process=subprocess.Popen(command,stdout=log,stderr=log);wait_ready(base,process)
            with sqlite3.connect(data/'blog.sqlite') as db:assert db.execute('SELECT COUNT(*) FROM wallpaper_deletions').fetchone()[0]==1
            assert sync()['state']=='failed'
            offline.rename(walls)
            assert sync()['state']=='succeeded' and third.read_bytes()==image and not staged.exists()
            # Never overwrite a newly created file at the recovery destination.
            staged=stage_deletion();third.write_bytes(b'replacement')
            assert admin.call('plugin-admin',{'id':'wallpapers','op':'delete','wall_id':third_id})[0]==422
            assert third.read_bytes()==b'replacement' and staged.read_bytes()==image
            third.unlink();assert sync()['state']=='succeeded'
            assert third.read_bytes()==image and not staged.exists()
            status,result=admin.call('plugin-admin',{'id':'wallpapers','op':'delete','wall_id':third_id})
            assert status==200 and result['deleted']==third.name and result['deleted_posts'],result
            assert not third.exists() and not list((data/'wallpaper-thumbs').glob(third_id+'-*'))
            assert all(w['id']!=third_id for w in gallery()['items'])
            # Committed deletion interrupted before physical cleanup finishes.
            original_id=hashlib.sha256(original.name.encode()).hexdigest()
            process.terminate();process.wait(timeout=15)
            metadata=original.stat();rootmeta=walls.stat();staged=walls/'.deleted-committed'
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute('INSERT INTO wallpaper_deletions VALUES(?,?,?,?,?,?,?,?)',(original_id,str(walls),original.name,staged.name,rootmeta.st_dev,rootmeta.st_ino,metadata.st_dev,metadata.st_ino))
                db.execute('DELETE FROM wallpapers WHERE id=?',(original_id,))
            original.rename(staged)
            process=subprocess.Popen(command,stdout=log,stderr=log);wait_ready(base,process)
            assert not staged.exists() and not original.exists()
            assert not list((data/'wallpaper-thumbs').glob(original_id+'-*'))
            with sqlite3.connect(data/'blog.sqlite') as db:assert db.execute('SELECT COUNT(*) FROM wallpaper_deletions').fetchone()[0]==0
            # Multipart batches preserve orientation, names, atomic posts and image originals.
            def upload_batch(client,files,count=None):
                boundary='wallpaper-test-boundary'
                body=b''
                for key,value in {'id':'wallpapers','op':'upload','count':len(files) if count is None else count}.items():
                    body+=f'--{boundary}\r\nContent-Disposition: form-data; name="{key}"\r\n\r\n{value}\r\n'.encode()
                for name,content in files:
                    body+=f'--{boundary}\r\nContent-Disposition: form-data; name="images[]"; filename="{name}"\r\nContent-Type: image/png\r\n\r\n'.encode()+content+b'\r\n'
                body+=f'--{boundary}--\r\n'.encode()
                request=urllib.request.Request(base+'/api?action=plugin-admin',data=body,headers={'Content-Type':'multipart/form-data; boundary='+boundary,'X-CSRF-Token':client.csrf})
                try:
                    with client.http.open(request,timeout=20) as response:return response.status,json.load(response)
                except urllib.error.HTTPError as error:return error.code,json.load(error)
            assert upload_batch(guest,[('land.png',image)])[0]==401
            assert upload_batch(admin,[('land.png',image)],2)[0]==422
            portrait=root/'portrait.png'
            subprocess.run(['magick','-size','200x400','gradient:red-blue',str(portrait)],check=True)
            status,result=upload_batch(admin,[('land.png',image),('land.png',image),('竖屏.png',portrait.read_bytes())])
            assert status==200,result
            uploaded=result['uploaded'];assert len(uploaded)==len(set(uploaded))==3
            assert uploaded[0]=='land.png' and uploaded[1].startswith('land-') and uploaded[2]=='Portrait/竖屏.png',uploaded
            posts=guest.call('posts')[1]['posts'];assert len(posts)==1 and posts[0]['content']=='收了 3 张新壁纸'
            assert len(posts[0]['images'])==3 and posts[0]['source_key'].startswith('wall-upload:')
            for descriptor in posts[0]['images']:
                response=guest.http.open(base+descriptor['thumb']);assert response.headers['Content-Type']=='image/webp'
                assert response.read()[:4]==b'RIFF'
            assert (walls/'land.png').read_bytes()==image
            assert not list(walls.glob('.upload-*'))
            assert sync()['result']['added']==0
            assert upload_batch(admin,[('valid.png',image),('invalid.png',b'invalid image')])[0]==422
            assert not (walls/'valid.png').exists() and not list(walls.glob('.upload-*'))
            with sqlite3.connect(data/'blog.sqlite') as db:
                assert db.execute('SELECT COUNT(*) FROM wallpaper_uploads').fetchone()[0]==0
                db.execute("CREATE TRIGGER fail_upload BEFORE INSERT ON posts WHEN NEW.source='wallpapers' BEGIN SELECT RAISE(ABORT,'fixture failure'); END")
            assert upload_batch(admin,[('rollback.png',image)])[0]==500
            assert not (walls/'rollback.png').exists() and not list(walls.glob('.upload-*'))
            assert len(gallery()['items'])==3
            with sqlite3.connect(data/'blog.sqlite') as db:db.execute('DROP TRIGGER fail_upload')
            # Thumbnail publication failure after commit must not report an unsuccessful upload.
            cache_dir=data/'wallpaper-thumbs';saved_cache=root/'saved-cache';cache_dir.rename(saved_cache)
            cache_dir.write_bytes(b'fixture blocks cache directory')
            status,result=upload_batch(admin,[('deferred.png',image)])
            assert status==200 and (walls/'deferred.png').read_bytes()==image,result
            with sqlite3.connect(data/'blog.sqlite') as db:assert db.execute('SELECT COUNT(*) FROM wallpaper_uploads WHERE committed=1').fetchone()[0]==1
            cache_dir.unlink();saved_cache.rename(cache_dir)
            assert sync()['state']=='succeeded'
            with sqlite3.connect(data/'blog.sqlite') as db:assert db.execute('SELECT COUNT(*) FROM wallpaper_uploads').fetchone()[0]==0
            assert admin.call('plugin-admin',{'id':'wallpapers','op':'delete','wall_id':hashlib.sha256(b'deferred.png').hexdigest()})[0]==200
            # Portrait placement may not escape through an external symlink.
            retained=root/'retained-portrait';(walls/'Portrait').rename(retained)
            external=root/'external';external.mkdir();(walls/'Portrait').symlink_to(external,target_is_directory=True)
            assert upload_batch(admin,[('escape.png',portrait.read_bytes())])[0]==422
            assert not list(external.iterdir()) and not list(walls.glob('.upload-*'))
            (walls/'Portrait').unlink();retained.rename(walls/'Portrait')
            browser_image=root/'browser.png';browser_image.write_bytes(image)
            subprocess.run(['node','tests/rust/wallpaper-upload-browser.cjs',base,str(browser_image)],cwd=ROOT,check=True)
            assert not (walls/'browser.png').exists() and len(gallery()['items'])==3
            # A prepared or linked but uncommitted upload is removed on restart.
            process.terminate();process.wait(timeout=15)
            rootmeta=walls.stat();staged=walls/'.upload-interrupted';staged.write_bytes(image)
            linked=walls/'interrupted.png';os.link(staged,linked)
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute('INSERT INTO wallpaper_uploads(token,root,path,staged,root_dev,root_ino) VALUES(?,?,?,?,?,?)',('interrupted',str(walls),linked.name,staged.name,rootmeta.st_dev,rootmeta.st_ino))
            process=subprocess.Popen(command,stdout=log,stderr=log);wait_ready(base,process)
            assert not staged.exists() and not linked.exists()
            # A different file at the final path is never treated as our failed upload.
            process.terminate();process.wait(timeout=15)
            staged.write_bytes(image);linked.write_bytes(b'unrelated')
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute('INSERT INTO wallpaper_uploads(token,root,path,staged,root_dev,root_ino) VALUES(?,?,?,?,?,?)',('interrupted',str(walls),linked.name,staged.name,rootmeta.st_dev,rootmeta.st_ino))
            process=subprocess.Popen(command,stdout=log,stderr=log);wait_ready(base,process)
            assert linked.read_bytes()==b'unrelated' and not staged.exists();linked.unlink()
            # Committed originals survive; recovery finishes publishing the staged thumbnail.
            process.terminate();process.wait(timeout=15)
            landed=walls/'land.png';wallid=hashlib.sha256(b'land.png').hexdigest()
            cache=next((data/'wallpaper-thumbs').glob(wallid+'-*'));thumbnail=cache.read_bytes();cache.unlink()
            os.link(landed,staged);staged_thumb=walls/(staged.name+'.webp');staged_thumb.write_bytes(thumbnail)
            with sqlite3.connect(data/'blog.sqlite') as db:
                db.execute('INSERT INTO wallpaper_uploads(token,root,path,staged,cache,committed,root_dev,root_ino) VALUES(?,?,?,?,?,1,?,?)',('committed',str(walls),landed.name,staged.name,cache.name,rootmeta.st_dev,rootmeta.st_ino))
            process=subprocess.Popen(command,stdout=log,stderr=log);wait_ready(base,process)
            assert landed.read_bytes()==image and cache.read_bytes()==thumbnail
            assert not staged.exists() and not staged_thumb.exists()
            with sqlite3.connect(data/'blog.sqlite') as db:assert db.execute('SELECT COUNT(*) FROM wallpaper_uploads').fetchone()[0]==0
            print('PASS: wallpaper baseline, scan/rollback/offline protection, references/thumbnails, gallery, media boundary, removal, tombstones, cancellation, seconds schedule, administrator deletion, multipart uploads and interrupted file operation recovery')
        except BaseException:
            log.flush();log.seek(0);print(log.read());raise
        finally:
            process.terminate();process.wait(timeout=15);log.close()

if __name__=='__main__':main()

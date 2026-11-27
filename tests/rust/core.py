#!/usr/bin/env python3
import http.cookiejar
import io
import zipfile
import concurrent.futures
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from contract import ROOT, free_port, wait_ready


class Client:
    def __init__(self, base):
        self.base = base
        self.jar = http.cookiejar.CookieJar()
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(self.jar))
        self.csrf = None
    def call(self, action, data=None, **query):
        headers = {}
        if data is not None:
            headers = {'Content-Type':'application/json','X-CSRF-Token':self.csrf or ''}
        request=urllib.request.Request(self.base+'/api?'+urllib.parse.urlencode({'action':action,**query}),data=None if data is None else json.dumps(data).encode(),headers=headers)
        try:
            with self.http.open(request,timeout=20) as response:
                return response.status,json.load(response)
        except urllib.error.HTTPError as error:
            return error.code,json.load(error)
    def state(self):
        status,body=self.call('state')
        assert status==200,body
        self.csrf=body['csrf']
        return body
    def upload(self, action, field, path, fields=None):
        boundary='muke-test-boundary'
        body=b''
        for key,value in (fields or {}).items():
            body+=f'--{boundary}\r\nContent-Disposition: form-data; name="{key}"\r\n\r\n{value}\r\n'.encode()
        body+=f'--{boundary}\r\nContent-Disposition: form-data; name="{field}"; filename="test.png"\r\nContent-Type: image/png\r\n\r\n'.encode()+path.read_bytes()+f'\r\n--{boundary}--\r\n'.encode()
        req=urllib.request.Request(self.base+'/api?action='+action,data=body,headers={'Content-Type':'multipart/form-data; boundary='+boundary,'X-CSRF-Token':self.csrf})
        with self.http.open(req,timeout=20) as response:return json.load(response)


def main():
    with tempfile.TemporaryDirectory(prefix='muke-core-') as temporary:
        root=Path(temporary)
        data=root/'data'
        base=f'http://127.0.0.1:{free_port()}'
        command=[str(ROOT/'target/debug/muke'),'--data-dir',str(data),'serve','--listen',base.removeprefix('http://'),'--frontend',str(ROOT/'frontend')]
        process=subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,cwd=temporary)
        try:
            wait_ready(base,process)
            admin=Client(base);guest=Client(base)
            state=admin.state();assert state['setup'] and not state['admin']
            old_cookie=next(iter(admin.jar)).value
            assert admin.call('setup',{'username':'tester','password':'Password12345'})[0]==200
            assert next(iter(admin.jar)).value!=old_cookie
            state=admin.state();assert state['admin'] and not state['setup']
            guest.state()
            assert guest.call('save-article',{'title':'x','content':'y'})[0]==401
            assert guest.call('setup',{'username':'other','password':'Password12345'})[0]==409
            csrf=admin.csrf;admin.csrf='bad'
            assert admin.call('save-post',{'content':'bad csrf'})[0]==403
            admin.csrf=csrf
            assert admin.call('save-post',{'content':'hello'})[0]==200
            post=admin.call('posts')[1]['posts'][0]
            assert admin.call('pin-post',{'id':post['id'],'pinned':1})[0]==200
            assert admin.call('posts')[1]['posts'][0]['pinned']==1
            comment={'kind':'post','target':post['id'],'author':'Visitor','email':'private@example.com','content':'hello comment'}
            assert guest.call('comment',comment)[0]==200
            rows=guest.call('comments',kind='post',target=post['id'])[1]['comments']
            assert len(rows)==1 and 'email' not in rows[0] and len(rows[0]['email_hash'])==32
            cid=rows[0]['id']
            assert guest.call('moderate-comment',{'id':cid,'op':'spam'})[0]==401
            assert admin.call('moderate-comment',{'id':cid,'op':'spam'})[0]==200
            assert guest.call('comment',dict(comment,content='automatically pending'))[0]==200
            assert guest.call('comments',kind='post',target=post['id'])[1]['comments']==[]
            moderation=admin.call('comments-admin')[1]
            assert moderation['counts']['pending']==1 and 'email' not in moderation['comments'][0]
            pending=moderation['comments'][0]['id']
            assert admin.call('moderate-comment',{'id':pending,'op':'approve'})[0]==200
            assert len(guest.call('comments',kind='post',target=post['id'])[1]['comments'])==1
            assert guest.call('comment',dict(comment,website_url='bot'))[0]==200
            assert admin.call('comments-admin')[1]['counts']['all']==2
            token='a'*32
            assert guest.call('like',{'id':post['id'],'token':token})[1]['liked']
            assert guest.call('likes',token=token)[1]['liked']==[post['id']]
            assert not guest.call('like',{'id':post['id'],'token':token})[1]['liked']
            assert admin.call('save-settings',{'comment_provider':'off'})[0]==200
            assert guest.call('comment',comment)[0]==422
            assert admin.call('save-settings',{'comment_provider':''})[0]==200
            uploaded=admin.upload('upload-image','image',ROOT/'tests/fixtures/rss/pixel.png')
            image_name=uploaded['url'].split('=')[1]
            body={'title':'Article','content':'initial ![picture]('+uploaded['url']+')','format':'md','tags':'猫，测试 猫','draft':1}
            assert admin.call('save-article',body)[0]==200
            article=admin.call('articles',draft=1)[1]['articles'][0]
            aid=article['id'];assert article['tags']=='猫,测试'
            assert guest.call('articles',id=aid)[0]==404
            assert guest.call('articles',draft=1)[0]==401
            assert guest.call('article-revisions',id=aid)[0]==401
            assert admin.call('articles',id=aid)[0]==200
            body.update(id=aid,content='published',draft=0,format='html')
            assert admin.call('save-article',body)[0]==200
            assert admin.call('articles',id=aid)[1]['article']['format']=='md'
            revisions=admin.call('article-revisions',id=aid)[1]['revisions']
            assert len(revisions)==1
            assert (data/'uploads'/image_name).is_file()
            assert admin.call('save-article',body)[0]==200
            assert len(admin.call('article-revisions',id=aid)[1]['revisions'])==1
            assert admin.call('restore-article-revision',{'id':aid,'revision':revisions[0]['id']})[0]==200
            assert guest.call('articles',id=aid)[0]==404
            assert len(admin.call('article-revisions',id=aid)[1]['revisions'])==2
            assert admin.call('delete-article',{'id':aid})[0]==200
            assert not (data/'uploads'/image_name).exists()
            assert admin.call('article-revisions',id=aid)[0]==404
            assert admin.upload('save-post','images[]',ROOT/'tests/fixtures/rss/pixel.png',{'content':'photo'})['ok']
            image_post=admin.call('posts')[1]['posts'][1]
            assert image_post['images']
            filename=image_post['images'][0]['ref'][3:]
            assert admin.call('delete-post',{'id':image_post['id']})[0]==200
            assert not (data/'uploads'/filename).exists()
            before=admin.state()['options']['site_tagline']
            assert admin.call('save-settings',{'site_tagline':'must roll back','post_max_chars':0})[0]==422
            assert admin.state()['options']['site_tagline']==before
            assert admin.call('save-settings',{'post_max_chars':'2','smtp_pass':'private secret'})[0]==200
            assert 'private secret' not in json.dumps(guest.state())
            assert admin.call('save-post',{'content':'long'})[0]==422
            assert admin.call('profile',{'name':'New name','bio':'My bio','handle':'new_handle'})[0]==200
            assert guest.state()['profile']['name']=='New name'
            # Profile images are cropped to WebP and replaced atomically.
            profile_image=root/'profile.png'
            subprocess.run(['magick','-size','900x600','gradient:red-blue',str(profile_image)],check=True)
            assert guest.call('save-avatar',{})[0]==401
            for kind,dimensions in [('avatar','400 400'),('cover','1500 500')]:
                result=admin.upload('save-'+kind,kind,profile_image)
                assert result['ok'] and result[kind].startswith('/image?'+kind+'=1&v=')
                assert guest.state()['profile'][kind]==result[kind]
                response=guest.http.open(base+result[kind]);assert response.headers['Content-Type']=='image/webp'
                stored=data/(kind+'.webp');assert response.read()==stored.read_bytes()
                assert subprocess.check_output(['magick',str(stored),'-format','%w %h','info:'],text=True)==dimensions
                before_bytes=stored.read_bytes();invalid=root/'invalid.png';invalid.write_bytes(b'not an image')
                try:admin.upload('save-'+kind,kind,invalid);raise AssertionError('invalid image accepted')
                except urllib.error.HTTPError as error:assert error.code==422
                assert stored.read_bytes()==before_bytes
                assert not list((data/'.media-staging').iterdir())
            # Generic thumbnails cache one result for concurrent requests and follow settings.
            uploaded=admin.upload('upload-image','image',profile_image)
            name=uploaded['url'].split('=')[1];thumb_url=base+uploaded['url']+'&thumb=1'
            def fetch_thumb(_=None):
                with guest.http.open(thumb_url) as response:
                    assert response.headers['Content-Type']=='image/webp'
                    return response.read()
            with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
                thumbs=list(executor.map(fetch_thumb,range(8)))
            assert all(data==thumbs[0] for data in thumbs)
            directory=data/'thumbs'/name;cached=list(directory.iterdir());assert len(cached)==1
            assert subprocess.check_output(['magick',str(cached[0]),'-format','%w %h','info:'],text=True)=='720 480'
            before_mtime=cached[0].stat().st_mtime_ns
            assert fetch_thumb()==thumbs[0] and cached[0].stat().st_mtime_ns==before_mtime
            assert admin.call('save-settings',{'thumb_size':'300','thumb_quality':'60'})[0]==200
            fetch_thumb();new_cache=list(directory.iterdir());assert len(new_cache)==1 and new_cache[0]!=cached[0]
            assert subprocess.check_output(['magick',str(new_cache[0]),'-format','%w %h','info:'],text=True)=='300 200'
            # Cleanup follows article references, including the thumbnail cache directory.
            status,article=admin.call('save-article',{'title':'Thumbnail cleanup','content':'![image]('+uploaded['url']+')','format':'md','draft':0})
            assert status==200,article
            article=next(a for a in admin.call('articles')[1]['articles'] if a['title']=='Thumbnail cleanup')
            assert admin.call('delete-article',{'id':article['id']})[0]==200
            assert not directory.exists() and not (data/'uploads'/name).exists()
            # AVIF remains visible in post descriptors and uses the same thumbnail pipeline.
            avif=root/'photo.avif'
            subprocess.run(['magick','-size','240x160','gradient:blue-green',str(avif)],check=True)
            assert admin.upload('save-post','images[]',avif,{'content':'图'})['ok']
            avif_post=next(p for p in admin.call('posts')[1]['posts'] if any(i['ref'].endswith('.avif') for i in p['images']))
            descriptor=avif_post['images'][0]
            assert descriptor['width']==240 and descriptor['height']==160 and descriptor['bytes']==avif.stat().st_size
            response=guest.http.open(base+descriptor['url']);assert response.headers['Content-Type']=='image/avif' and response.read()==avif.read_bytes()
            response=guest.http.open(base+descriptor['thumb']);assert response.headers['Content-Type']=='image/webp' and response.read()[:4]==b'RIFF'
            avif_name=descriptor['ref'][3:]
            assert admin.call('delete-post',{'id':avif_post['id']})[0]==200
            assert not (data/'uploads'/avif_name).exists() and not (data/'thumbs'/avif_name).exists()
            # Cached Gravatar responses work through the public endpoint without contacting upstream.
            gravatar_hash='1234567890abcdef1234567890abcdef'
            gravatars=data/'gravatar';gravatars.mkdir(exist_ok=True)
            (gravatars/gravatar_hash).write_bytes(profile_image.read_bytes())
            response=guest.http.open(base+'/image?gravatar='+gravatar_hash)
            assert response.headers['Content-Type']=='image/png' and response.read()==profile_image.read_bytes()
            try:guest.http.open(base+'/image?gravatar=invalid');raise AssertionError('invalid Gravatar hash accepted')
            except urllib.error.HTTPError as error:assert error.code==404
            first_avatar=guest.state()['profile']['avatar']
            assert admin.call('save-settings',{'avatar_size':'200'})[0]==200
            assert admin.upload('save-avatar','avatar',profile_image)['avatar']!=first_avatar
            assert subprocess.check_output(['magick',str(data/'avatar.webp'),'-format','%w %h','info:'],text=True)=='200 200'
            for kind in ['avatar','cover']:
                assert admin.call('remove-'+kind,{})[0]==200
                assert guest.state()['profile'][kind] is None
                try:guest.http.open(base+'/image?'+kind+'=1');raise AssertionError('removed profile image served')
                except urllib.error.HTTPError as error:assert error.code==404
            private=root/'private.webp';private.write_bytes(b'private')
            (data/'avatar.webp').symlink_to(private)
            assert guest.state()['profile']['avatar'] is None
            try:guest.http.open(base+'/image?avatar=1');raise AssertionError('private symlink served')
            except urllib.error.HTTPError as error:assert error.code==404
            assert admin.call('remove-avatar',{})[0]==200 and private.read_bytes()==b'private'
            # 静态导出走统一后台任务：管理员提交、去重、产物原子发布到服务工作目录 dist/
            assert guest.call('export',{})[0]==401
            assert admin.call('save-article',{'title':'Export check','content':'exported body','format':'md','draft':0})[0]==200
            exported=admin.call('articles')[1]['articles'][0]
            task=admin.call('export',{})[1]['task_id']
            assert admin.call('export',{})[1]['task_id']==task
            for _ in range(300):
                jobs=admin.call('tasks')[1]['jobs']
                job=next(job for job in jobs if job['id']==task)
                if job['state'] not in ('queued','running'):break
                time.sleep(0.1)
            else:raise AssertionError('export task timed out')
            assert job['state']=='succeeded',job
            assert job['result']['posts']>=1 and job['result']['articles']>=1
            dist=root/'dist'
            assert (dist/'index.html').is_file() and (dist/'rss.xml').is_file() and (dist/'sitemap.xml').is_file() and (dist/'_headers').is_file()
            assert 'hello' in (dist/'index.html').read_text()
            assert 'exported body' in (dist/'article'/str(exported['id'])/'index.html').read_text()
            assert not [entry for entry in root.iterdir() if entry.name.startswith('.export-')]
            # 站点包下载：缓存复用与 zip 内容
            try:guest.http.open(base+'/api?action=download-static');raise AssertionError('guest downloaded static zip')
            except urllib.error.HTTPError as error:assert error.code==401
            with admin.http.open(base+'/api?action=download-static') as response:
                assert response.headers['Content-Type']=='application/zip'
                archive=zipfile.ZipFile(io.BytesIO(response.read()))
            assert 'index.html' in archive.namelist() and 'rss.xml' in archive.namelist()
            assert 'hello' in archive.read('index.html').decode()
            cached=(data/'static-site.zip').stat().st_mtime_ns
            with admin.http.open(base+'/api?action=download-static') as response:assert len(response.read())>0
            assert (data/'static-site.zip').stat().st_mtime_ns==cached
            # Netlify 部署配置与状态：不触发真实网络请求
            status,payload=admin.call('deploy-status')
            assert status==200 and payload['status'] is None and not payload['cancelling'] and payload['pending'] is None
            assert guest.call('deploy-status')[0]==401
            assert admin.call('cancel-deploy',{})[0]==409
            assert admin.call('netlify-deploy',{})[0]==422
            assert admin.call('netlify-save',{'token':'x','site':'bad site!'})[0]==422
            assert admin.call('netlify-save',{'token':'secret-token','site':'my-site-123'})[0]==200
            state=admin.state()
            assert state['netlify']['site']=='my-site-123' and state['netlify']['has_token'] and state['netlify']['auto_deploy'] is False
            assert admin.call('save-settings',{'auto_deploy':'1','auto_deploy_debounce':'3'})[0]==200
            state=admin.state()
            assert state['netlify']['auto_deploy'] is True and state['netlify']['auto_deploy_debounce']==3
            # 新动态触碰自动部署防抖（凭据已备、开关已开）
            assert admin.call('save-settings',{'post_max_chars':'5000'})[0]==200
            status,payload=admin.call('save-post',{'content':'auto deploy touch'})
            assert status==200,payload
            status,payload=admin.call('deploy-status')
            assert payload['pending'] is not None and 0<payload['pending']['in']<=180
            # 手动清空凭据
            assert admin.call('netlify-save',{'token':'','site':''})[0]==200
            state=admin.state()
            assert state['netlify']['site']=='' and not state['netlify']['has_token']
            # JSON 备份导出/导入整体替换（登录账号保留）
            assert guest.call('backup-export')[0]==401
            backup=admin.call('backup-export')[1]
            assert backup['app']=='mkblog' and any(post['content'].endswith('auto deploy touch') for post in backup['posts'])
            assert admin.call('backup-import',{'app':'other'})[0]==422
            backup['posts'].append({'id':9999,'content':'导入的动态','images':'[]','meta':'','created_at':'2026-11-26T09:00:00+08:00','updated_at':'2026-11-26T09:00:00+08:00'})
            backup['settings']['site_tagline']='导入后的标语'
            status,imported=admin.call('backup-import',backup)
            assert status==200,imported
            assert imported['ok'] and imported['posts']==len(backup['posts'])
            posts=admin.call('posts')[1]['posts']
            assert any(post['content']=='导入的动态' for post in posts),len(posts)
            assert guest.state()['options']['site_tagline']=='导入后的标语'
            state=admin.state();assert state['admin'],'导入保留登录账号'
            second=Client(base);second.state()
            assert second.call('login',{'username':'tester','password':'Password12345'})[0]==200
            second.state()
            assert admin.call('change-password',{'current':'Password12345','next':'ChangedPassword123'})[0]==200
            assert not second.state()['admin']
            assert admin.call('logout',{})[0]==200
            assert not admin.state()['admin']
            assert admin.call('login',{'username':'tester','password':'Password12345'})[0]==401
            assert admin.call('login',{'username':'tester','password':'ChangedPassword123'})[0]==200
            assert admin.state()['admin']
            # Existing bcrypt hashes with the $2y$ prefix remain valid.
            legacy='$2y$12$wbNaekzw5R/T3RfFWvR.POeZhIKcmziE/JDgxUjJq4.AyIxh.Z1ly'
            with sqlite3.connect(data/'blog.sqlite') as db:db.execute('UPDATE users SET password=? WHERE id=1',(legacy,))
            third=Client(base);third.state()
            assert third.call('login',{'username':'tester','password':'LegacyPassword123'})[0]==200
            process.terminate();process.wait(timeout=10)
            abandoned=data/'.media-staging'/'profile-abandoned';abandoned.mkdir();(abandoned/'source').write_bytes(b'incomplete')
            process=subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,cwd=temporary)
            wait_ready(base,process)
            assert not abandoned.exists()
            assert third.state()['admin'], 'session should survive server restart'
            subprocess.run(['node',str(ROOT/'tests/rust/browser.cjs'),base,'LegacyPassword123'],check=True)
            subprocess.run(['node',str(ROOT/'tests/rust/profile-browser.cjs'),base,'LegacyPassword123',str(profile_image)],check=True)
            limited=Client(base);limited.state()
            for _ in range(5):assert limited.call('login',{'username':'tester','password':'wrong'})[0]==401
            assert limited.call('login',{'username':'tester','password':'wrong'})[0]==429
            assert guest.call('articles',id='x')[0]==404
            print('PASS: migrations, setup/login/CSRF, persistent sessions, bcrypt migration, throttling, password revocation, content CRUD, uploads, draft privacy, revision restore, image lifetime, atomic settings')
        finally:
            if process.poll() is None:process.terminate();process.wait(timeout=10)
            error=process.stderr.read()
            if error:print(error[-4000:])

if __name__=='__main__':main()

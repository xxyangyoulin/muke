#!/usr/bin/env python3
"""Verify read APIs and legacy migration against fixed, synthetic fixtures."""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import sqlite3
import statistics
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def request(base, params):
    url = base + '/api?' + urllib.parse.urlencode(params)
    try:
        with HTTP.open(url, timeout=10) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, json.load(error)


def wait_ready(base, process):
    for _ in range(100):
        if process.poll() is not None:
            raise RuntimeError('server exited: ' + process.stdout.read())
        try:
            if request(base, {'action': 'posts', 'limit': 1})[0] == 200:
                return
        except (OSError, ValueError):
            pass
        time.sleep(.1)
    raise RuntimeError('server startup timed out')


def free_port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


def benchmark(base, pid):
    durations = []
    peak = [0]
    def rss_tree(parent):
        try:
            status = Path(f'/proc/{parent}/status').read_text()
            rss = next(int(line.split()[1]) for line in status.splitlines() if line.startswith('VmRSS:'))
            children = {child for path in Path(f'/proc/{parent}/task').glob('*/children') for child in path.read_text().split()}
            return rss + sum(rss_tree(child) for child in children)
        except (OSError, StopIteration):
            return 0
    def one(_):
        start = time.perf_counter()
        assert request(base, {'action': 'posts', 'limit': 30})[0] == 200
        peak[0] = max(peak[0], rss_tree(pid))
        return (time.perf_counter() - start) * 1000
    for _ in range(5):
        one(0)
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
        durations = list(executor.map(one, range(80)))
    durations.sort()
    return {'requests': 80, 'concurrency': 4, 'median_ms': round(statistics.median(durations), 2), 'p95_ms': round(durations[int(len(durations)*.95)-1], 2), 'sampled_peak_rss_tree_kib': peak[0]}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--report', type=Path)
    parser.add_argument('--keep', type=Path, help='Create the fixture in a new directory and retain it')
    args = parser.parse_args()
    temporary = tempfile.TemporaryDirectory(prefix='muke-rust-') if not args.keep else None
    root = Path(temporary.name) if temporary else args.keep.resolve()
    if args.keep:
        root.mkdir(parents=True, exist_ok=False)
    processes = []
    try:
        data = root / 'data'
        (data/'uploads').mkdir(parents=True)
        with sqlite3.connect(data/'blog.sqlite') as db:
            db.executescript((ROOT/'tests/fixtures/legacy.sql').read_text())
        shutil.copyfile(ROOT/'tests/fixtures/rss/pixel.png',data/'uploads/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png')
        expected = json.loads((ROOT/'tests/fixtures/read-api.json').read_text())
        env = {key:value for key,value in os.environ.items() if not key.startswith('MKBLOG_')}
        config = root/'config.toml';config.write_text('')
        binary = ROOT/'target/debug/muke'
        command = [str(binary), '--config', str(config), '--data-dir', str(data), '--work-dir', str(root/'work')]
        rust_url = f'http://127.0.0.1:{free_port()}'
        def content_snapshot():
            with sqlite3.connect(data/'blog.sqlite') as db:
                return {table:db.execute('SELECT * FROM '+table+' ORDER BY rowid').fetchall() for table in ['posts','articles','article_revisions','comments','post_likes','settings']}
        before = content_snapshot()
        def start_rust(extra_env=None):
            p = subprocess.Popen(command+['serve','--listen',rust_url.removeprefix('http://'),'--frontend',str(ROOT/'frontend')], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=extra_env or env, cwd=root)
            processes.append(p)
            wait_ready(rust_url,p)
            return p
        rust = start_rust()
        def cli(*parts):
            return subprocess.check_output(command+list(parts),text=True,env=env,cwd=root).strip()
        for case in expected['cases']:
            assert request(rust_url,case['query']) == (case['status'],case['body']),case['query']
        for action in ['posts','articles']:
            cursor = None
            seen = []
            for left in expected['pages'][action]:
                case = {'action':action,'limit':2}
                if cursor: case['cursor']=cursor
                right_status,right=request(rust_url,case)
                assert right_status==200
                if action=='articles':
                    for values in [left,right]:
                        for item in values[action]:
                            # Rendering has its own fixture coverage.
                            item.pop('excerpt');item.pop('text_length')
                left_cursor=left.pop('next_cursor');right_cursor=right.pop('next_cursor')
                if left_cursor:
                    import base64
                    assert json.loads(base64.b64decode(left_cursor))==json.loads(base64.b64decode(right_cursor))
                else: assert right_cursor is None
                assert left==right,(case,left,right)
                seen.extend(item['id'] for item in right[action])
                cursor=left_cursor
            assert cursor is None
            assert len(seen)==len(set(seen))
            assert len(seen)==(64 if action=='posts' else 4)
        for case in expected['searches']:
            status,body=request(rust_url,case['query'])
            assert status==200 and body['total']==case['total']
            assert [item['id'] for item in body['articles']]==case['ids']
        safe=request(rust_url,{'action':'articles','id':5})[1]['article']['html']
        assert 'onerror' not in safe and 'javascript:' not in safe
        subprocess.run(['node',str(ROOT/'tests/rust/browser.cjs'),rust_url],check=True)
        source=data/'uploads/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png'
        image_url=rust_url+'/image?upload='+source.name
        assert HTTP.open(image_url).read()==source.read_bytes()
        try:
            HTTP.open(rust_url+'/image?upload=../../blog.sqlite')
            raise AssertionError('path traversal accepted')
        except urllib.error.HTTPError as error: assert error.code==404
        try:
            HTTP.open(urllib.request.Request(rust_url+'/api?action=save-post',data=b'{}'))
            raise AssertionError('read-only service accepted mutation')
        except urllib.error.HTTPError as error: assert error.code==403
        duplicate=subprocess.run(command+['serve','--listen','127.0.0.1:0','--frontend',str(ROOT/'frontend')],capture_output=True,text=True)
        assert duplicate.returncode!=0 and '已有服务' in duplicate.stderr
        def job(id):
            return next(j for j in json.loads(cli('tasks')) if j['id']==id)
        large=source.with_name('e'*32+'.png')
        subprocess.run(['magick','-seed','42','-size','1200x900','plasma:fractal',str(large)],check=True)
        assert large.stat().st_size>100*1024
        task_start=time.perf_counter()
        id=int(cli('thumbnail',str(large)))
        for _ in range(200):
            if job(id)['state'] in ['succeeded','failed']: break
            time.sleep(.05)
        assert job(id)['state']=='succeeded',job(id)
        real_compression_ms=round((time.perf_counter()-task_start)*1000,2)
        output=root/'work'/f'image-{id}'
        assert 0<(output/'image.webp').stat().st_size<100*1024
        assert 0<(output/'thumb.webp').stat().st_size<50*1024
        rust.terminate();rust.wait(timeout=10)
        queued=int(cli('thumbnail',str(source)))
        assert int(cli('thumbnail',str(source)))==queued
        assert cli('cancel',str(queued))=='true' and job(queued)['state']=='cancelled'
        # Deterministic slow child exercises cancellation and SIGKILL, while the earlier
        # job checks real ImageMagick output and size limits.
        slow_bin=root/'slow-bin'
        slow_bin.mkdir()
        slow=slow_bin/'magick'
        slow.write_text('#!/usr/bin/python3\nimport os,sys,time\nfrom pathlib import Path\nPath(sys.argv[-1]).with_suffix(".pid").write_text(str(os.getpid()))\ntime.sleep(60)\n')
        slow.chmod(0o755)
        slow_env=dict(env,PATH=str(slow_bin)+os.pathsep+env['PATH'])
        ids=[]
        for letter in ['b','c','d']:
            image=source.with_name(letter*32+'.png')
            shutil.copyfile(source,image)
            ids.append(int(cli('thumbnail',str(image))))
        rust=start_rust(slow_env)
        def until(predicate):
            for _ in range(150):
                if predicate(): return
                time.sleep(.02)
            raise AssertionError('condition timeout')
        until(lambda: all((root/'work'/f'image-{i}'/'image.pid').exists() for i in ids[:2]))
        assert [job(i)['state'] for i in ids]==['running','running','queued']
        children=[int((root/'work'/f'image-{i}'/'image.pid').read_text()) for i in ids[:2]]
        loaded=benchmark(rust_url,rust.pid)
        assert cli('cancel',str(ids[2]))=='true'
        cancel_start=time.perf_counter()
        assert cli('cancel',str(ids[0]))=='true'
        until(lambda:job(ids[0])['state']=='cancelled')
        cancel_ms=round((time.perf_counter()-cancel_start)*1000,2)
        assert not Path(f'/proc/{children[0]}').exists()
        assert not (root/'work'/f'image-{ids[0]}').exists()
        rust.kill();rust.wait(timeout=10)
        def gone(pid):
            try: return Path(f'/proc/{pid}/stat').read_text().split()[2]=='Z'
            except FileNotFoundError: return True
        until(lambda:gone(children[1]))
        rust=start_rust()
        assert job(ids[1])['state']=='interrupted'
        assert job(ids[2])['state']=='cancelled'
        assert not (root/'work'/f'image-{ids[1]}').exists()
        rust.terminate();rust.wait(timeout=10)
        shutdown_ids=[int(cli('thumbnail',str(source.with_name(letter*32+'.png')))) for letter in ['b','c','d']]
        rust=start_rust(slow_env)
        until(lambda:all((root/'work'/f'image-{i}'/'image.pid').exists() for i in shutdown_ids[:2]))
        shutdown_pids=[int((root/'work'/f'image-{i}'/'image.pid').read_text()) for i in shutdown_ids[:2]]
        rust.terminate();rust.wait(timeout=10)
        assert [job(i)['state'] for i in shutdown_ids]==['cancelled','cancelled','queued']
        assert all(gone(pid) for pid in shutdown_pids)
        rust=start_rust()
        until(lambda:job(shutdown_ids[2])['state']=='succeeded')
        assert content_snapshot()==before
        report={'environment':{'os':platform.platform(),'cpu_count':os.cpu_count(),'cpu':next((line.split(':',1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),'unknown'),'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'rust_profile':'debug','fixture':{'posts':65,'published_articles':4,'drafts':1}},'rust':benchmark(rust_url,rust.pid),'rust_with_two_slow_children':loaded,'cancel_ms':cancel_ms,'real_compression_ms':real_compression_ms}
        if args.report:
            args.report.parent.mkdir(parents=True,exist_ok=True)
            args.report.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
        print(json.dumps(report,ensure_ascii=False,indent=2))
        print('PASS: API contracts, pagination, privacy, media, unchanged content tables, task deduplication/cancel/recovery, real image compression')
    finally:
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
                try: process.wait(timeout=10)
                except subprocess.TimeoutExpired: process.kill();process.wait()
        if temporary: temporary.cleanup()
        for process in processes:
            if process.stderr:
                diagnostics = process.stderr.read()
                if diagnostics: print(diagnostics[-4000:])

if __name__=='__main__': main()

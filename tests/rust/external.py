#!/usr/bin/env python3
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
from core import Client
from contract import ROOT, free_port, wait_ready


def until(check, seconds=90):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(.1)
    raise AssertionError('condition timed out')


def main():
    with tempfile.TemporaryDirectory(prefix='muke-external-') as temporary:
        root = Path(temporary)
        repository = root / 'plugin with spaces'
        shutil.copytree(ROOT / 'examples/standalone-plugin', repository, ignore=shutil.ignore_patterns('target', '.git'))
        subprocess.run(['git', 'init', '-q', str(repository)], check=True)
        module = repository / 'frontend/plugin.js'
        module.write_text("import { escapeMessage } from './format.js';\n" + module.read_text().replace('MKBLOG.esc(data.message)', 'escapeMessage(data.message)'))
        (repository / 'frontend/format.js').write_text('export function escapeMessage(text) { return MKBLOG.esc(text); }\n')
        original = (repository / 'src/main.rs').read_text()
        env = {key: value for key, value in os.environ.items() if not key.startswith('MKBLOG_')}
        env['CARGO_NET_OFFLINE'] = 'true'
        blocked_tmp = root / 'unusable-temp'
        blocked_tmp.write_text('not a directory')
        env['TMPDIR'] = str(blocked_tmp)
        (repository / 'build.rs').write_text('fn main() { std::fs::write(std::env::temp_dir().join("compiler-probe"), b"ok").unwrap(); }\n')

        env.setdefault('CARGO_HOME', str(Path.home() / '.cargo'))
        # Exercise the GitHub clone command and its arguments without external network.
        commands = root / 'commands'
        commands.mkdir()
        real_git = shutil.which('git')
        wrapper = commands / 'git'
        wrapper.write_text('#!/usr/bin/env python3\nimport os,sys,shutil,json\n'
            f'root={str(root)!r}\nsource={str(repository)!r}\nreal={real_git!r}\n'
            'if "clone" in sys.argv:\n'
            ' open(root+"/clone.json","w").write(json.dumps(sys.argv[1:]))\n'
            ' shutil.copytree(source,sys.argv[-1],ignore=shutil.ignore_patterns(".git","target"))\n'
            'else: os.execv(real,[real]+sys.argv[1:])\n')
        wrapper.chmod(0o755)
        env['PATH'] = str(commands) + os.pathsep + env['PATH']
        data = root / 'data'
        config = root / 'config.toml'
        config.write_text(f'export_dir = "{root / "dist"}"\n')
        binary = str(ROOT / 'target/debug/muke')
        base = f'http://127.0.0.1:{free_port()}'

        def start(directory):
            process = subprocess.Popen([binary, '--config', str(config), '--data-dir', str(directory), 'serve', '--listen', base[7:], '--frontend', str(ROOT / 'frontend')], env=env, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            wait_ready(base, process)
            return process

        process = start(data)
        try:
            admin, guest = Client(base), Client(base)
            admin.state(); guest.state()
            assert admin.call('setup', {'username':'tester', 'password':'Password12345'})[0] == 200
            admin.state()
            for action in ['plugin-environment', 'task-log']:
                assert guest.call(action, id=1)[0] == 401
            assert guest.call('install-plugin', {'source_kind':'local','source':str(repository)})[0] == 401
            assert admin.call('install-plugin', {'source_kind':'github','source':'https://github.com/a/b;touch owned'})[0] == 422
            assert admin.call('install-plugin', {'source_kind':'github','source':'https://example.org/a/b'})[0] == 422
            assert admin.call('install-plugin', {'source_kind':'local','source':'relative'})[0] == 422
            csrf = admin.csrf
            admin.csrf = 'invalid'
            assert admin.call('install-plugin', {'source_kind':'local','source':str(repository)})[0] == 403
            admin.csrf = csrf

            def job(task_id):
                return next(job for job in admin.call('tasks')[1]['jobs'] if job['id'] == task_id)

            def done(task_id):
                return until(lambda: job(task_id) if job(task_id)['state'] not in ['queued','running'] else None)

            def install(kind='local', source=None):
                status, result = admin.call('install-plugin', {'source_kind':kind,'source':source or str(repository)})
                assert status == 200, result
                return result['task_id']

            task = install()
            outcome = done(task)
            assert outcome['state'] == 'succeeded', (outcome, admin.call('task-log', id=task))
            assert outcome['result']['id'] == 'hello'
            assert 'Compiling' in admin.call('task-log', id=task)[1]['output']
            assert not (repository / 'target').exists()
            assert admin.call('save-plugin', {'id':'hello','enabled':1,'values':{'message':'Hello independent plugin'}})[0] == 200
            assert guest.call('plugin-data', id='hello')[1]['data']['message'] == 'Hello independent plugin'
            assert guest.http.open(base + '/plugins/hello/plugin.js').status == 200
            for path in ['/plugins/hello/../plugin', '/plugins/hello/plugin.json', '/plugins/hello/backend/mod.rs']:
                try:
                    guest.http.open(base + path)
                    raise AssertionError(path)
                except urllib.error.HTTPError as error:
                    assert error.code == 404
            task = admin.call('sync-plugin', {'id':'hello'})[1]['task_id']
            assert done(task)['result']['added'] == 1
            task = admin.call('sync-plugin', {'id':'hello'})[1]['task_id']
            assert done(task)['result']['added'] == 0
            assert guest.call('posts')[1]['posts'][0]['source'] == 'hello'
            assert admin.call('plugin-admin', {'id':'hello','state':{'saved':True}})[0] == 200
            subprocess.run(['node', str(ROOT / 'tests/rust/external-browser.cjs'), base, str(repository)], check=True)

            # Failed compilation and failed handshake both preserve the previous installation.
            (repository / 'src/main.rs').write_text('invalid rust code')
            task = install()
            assert done(task)['state'] == 'failed'
            assert guest.call('plugin-data', id='hello')[0] == 200
            (repository / 'src/main.rs').write_text(original.replace('"protocol":1,"id":"hello"', '"protocol":2,"id":"hello"'))
            assert done(install())['state'] == 'failed'
            assert guest.call('plugin-data', id='hello')[0] == 200
            (repository / 'src/main.rs').write_text(original)

            # Cancelling Cargo must also terminate its build-script descendants.
            child_pid = root / 'build-child.pid'
            (repository / 'build.rs').write_text('fn main() { let mut child = std::process::Command::new("sleep").arg("60").spawn().unwrap(); std::fs::write(' + json.dumps(str(child_pid)) + ',child.id().to_string()).unwrap(); child.wait().unwrap(); }')
            task = install()
            until(child_pid.exists)
            pid = int(child_pid.read_text())
            assert admin.call('cancel-task', {'id':task})[0] == 200
            assert done(task)['state'] == 'cancelled'
            def stopped():
                path = Path(f'/proc/{pid}/stat')
                return not path.exists() or path.read_text().split(') ')[1][0] == 'Z'
            until(stopped)
            (repository / 'build.rs').unlink()
            assert guest.call('plugin-data', id='hello')[0] == 200

            # Rebuild through the GitHub source path, then remove/reinstall and reload from disk.
            manifest = json.loads((repository / 'plugin.json').read_text())
            manifest['version'] = '0.2.0'
            (repository / 'plugin.json').write_text(json.dumps(manifest))
            for name in ['Cargo.toml', 'Cargo.lock']:
                path = repository / name
                path.write_text(path.read_text().replace('version = "0.1.0"', 'version = "0.2.0"', 1))
            export_pid = root / 'export-child.pid'
            (repository / 'src/main.rs').write_text(original.replace('Some("export") => {', 'Some("export") => { if request["state"]["slow_export"] == true { std::fs::write(' + json.dumps(str(export_pid)) + ', std::process::id().to_string()).unwrap(); std::thread::sleep(std::time::Duration::from_secs(60)); }'))
            task = install('github', 'https://github.com/example/independent-plugin')
            assert done(task)['state'] == 'succeeded', job(task)
            args = json.loads((root / 'clone.json').read_text())
            assert args[-2] == 'https://github.com/example/independent-plugin.git'
            assert '--' in args and '--depth' in args
            installed = next(plugin for plugin in admin.call('plugins')[1]['plugins'] if plugin['id'] == 'hello')
            assert installed['version'] == '0.2.0' and installed['enabled']
            assert admin.call('plugin-admin', {'id':'hello','state':{'slow_export':True}})[0] == 200
            task = admin.call('export', {})[1]['task_id']
            until(export_pid.exists)
            assert admin.call('cancel-task', {'id':task})[0] == 200
            outcome = until(lambda: job(task) if job(task)['state'] == 'cancelled' else None, seconds=5)
            pid = int(export_pid.read_text())
            until(stopped, seconds=5)
            assert admin.call('plugin-admin', {'id':'hello','state':{}})[0] == 200
            assert admin.call('remove-plugin', {'id':'hello','purge':False})[0] == 200
            assert done(install())['state'] == 'succeeded'
            installed = next(plugin for plugin in admin.call('plugins')[1]['plugins'] if plugin['id'] == 'hello')
            assert not installed['removed'] and not installed['enabled']
            assert admin.call('save-plugin', {'id':'hello','enabled':1,'values':{'message':'Hello independent plugin'}})[0] == 200
            task = admin.call('export', {})[1]['task_id']
            assert done(task)['state'] == 'succeeded', job(task)
            assert 'Hello independent plugin' in (root / 'dist/hello/index.html').read_text()
            assert not (root / 'dist/plugins/hello/plugin').exists()
            static_port = free_port()
            static_server = subprocess.Popen(['python3', '-m', 'http.server', str(static_port), '--bind', '127.0.0.1', '--directory', str(root / 'dist')], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                subprocess.run(['node', str(ROOT / 'tests/rust/external-browser.cjs'), f'http://127.0.0.1:{static_port}', '--static'], check=True)
            finally:
                static_server.terminate(); static_server.wait(timeout=10)
            process.terminate(); process.wait(timeout=10)
            backup = root / 'backup.zip'
            subprocess.run([binary, '--data-dir', str(data), 'backup', str(backup)], env=env, cwd=root, check=True)
            restored = root / 'restored'
            restored.mkdir()
            subprocess.run([binary, '--data-dir', str(data), 'restore', str(backup), str(restored)], env=env, cwd=root, check=True)
            with sqlite3.connect(restored / 'blog.sqlite') as db:
                assert db.execute('SELECT count(*) FROM plugin_installations').fetchone()[0] == 1
            process = start(restored)
            guest = Client(base); guest.state()
            assert guest.call('plugin-data', id='hello')[1]['data']['message'] == 'Hello independent plugin'
            process.terminate(); process.wait(timeout=10)
            with sqlite3.connect(restored / 'blog.sqlite') as db:
                installation = json.loads(db.execute('SELECT installation FROM plugin_installations').fetchone()[0])
            (restored / 'extensions/hello' / installation['generation'] / 'plugin').unlink()
            process = start(restored)
            admin = Client(base); admin.state()
            assert admin.call('login', {'username':'tester','password':'Password12345'})[0] == 200
            admin.state()
            assert done(install())['state'] == 'succeeded'
            assert admin.call('plugin-data', id='hello')[0] == 200
            print('PASS: independent local/GitHub source builds, authorization, logs, process-group cancellation, failed-update preservation, dynamic API/assets, static export, reinstall and backup restore')
        finally:
            if process.poll() is None:
                process.terminate(); process.wait(timeout=10)


if __name__ == '__main__':
    main()

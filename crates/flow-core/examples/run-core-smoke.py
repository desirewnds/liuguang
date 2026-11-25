# SPDX-License-Identifier: Apache-2.0
"""Only generated fixtures and owned processes; no system proxy discovery."""
from functools import partial
from http.server import ThreadingHTTPServer, SimpleHTTPRequestHandler
from pathlib import Path
import subprocess, threading, sys
root=Path(__file__).resolve().parents[3]
fixture=root/'test-artifacts'/'core-smoke'/'fixture'
fixture.mkdir(parents=True,exist_ok=True)
subprocess.run([str(root/'engine/ffmpeg.exe'),'-hide_banner','-loglevel','error','-y','-f','lavfi','-i','testsrc2=size=320x180:rate=24','-f','lavfi','-i','sine=frequency=440','-t','2','-c:v','libx264','-c:a','aac','-movflags','+faststart',str(fixture/'sample.mp4')],check=True)
child=fixture/'owned_child.py'
child.write_text('import os,sys,json,subprocess,time\nfrom pathlib import Path\np=subprocess.Popen([sys.executable,"-c","import time; time.sleep(90)"])\nPath(sys.argv[1]).write_text(json.dumps([os.getpid(),p.pid]))\np.wait()\n',encoding='utf8')
class Handler(SimpleHTTPRequestHandler):
 def log_message(self,*args): pass
server=ThreadingHTTPServer(('127.0.0.1',0),partial(Handler,directory=str(fixture)))
threading.Thread(target=server.serve_forever,daemon=True).start()
try:
 subprocess.run(['cargo','run','--offline','--manifest-path',str(root/'crates/flow-core/Cargo.toml'),'--example','core-smoke','--',str(root),'lifecycle',f'http://127.0.0.1:{server.server_port}',sys.executable,str(child)],check=True)
finally:
 server.shutdown();server.server_close()

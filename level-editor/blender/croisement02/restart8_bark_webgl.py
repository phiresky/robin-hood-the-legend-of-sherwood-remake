"""Run the approved bark actual-WebGL proof under the shared FIFO render lease."""
import sys,os,subprocess,threading,http.server
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
class Quiet(http.server.SimpleHTTPRequestHandler):
 def log_message(self,*args):pass
if __name__=='__main__':
 acquire();server=None
 try:
  server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Quiet);threading.Thread(target=server.serve_forever,daemon=True).start();subprocess.run(['node',str(Path(__file__).with_suffix('.mjs'))],check=True,env={**os.environ,'C02_TREE07_PORT':str(server.server_port),'TMPDIR':'/home/phire/.cache'})
 finally:
  if server:server.shutdown();server.server_close()
  release()

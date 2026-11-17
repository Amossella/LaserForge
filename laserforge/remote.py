"""Optional remote control from a phone on the local network (off by default, PIN protected).

A small web page shows the laser status and offers start / pause / resume / stop / frame / home / jog / air.
Commands are handed to the Qt main thread through a signal, so the server thread never touches the UI.
"""

import json
import socket
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

from PySide6.QtCore import QObject, Signal

PAGE = """<!doctype html><html lang="it"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>LaserForge</title>
<style>
body{font-family:-apple-system,Helvetica,sans-serif;background:#1f2023;color:#e6e6e6;margin:0;padding:16px}
h1{font-size:20px;margin:0 0 12px} .st{background:#2b2c30;border-radius:10px;padding:12px;margin-bottom:12px;font-size:15px}
.g{display:grid;grid-template-columns:repeat(3,1fr);gap:8px;margin-bottom:12px}
button{font-size:17px;padding:14px 6px;border:0;border-radius:10px;background:#3a3c42;color:#fff}
.go{background:#2f7bf5}.warn{background:#a7841f}.stop{background:#c62828}.air{background:#1b8a9b}
input{font-size:18px;padding:10px;border-radius:8px;border:1px solid #555;background:#111;color:#fff;width:140px}
</style></head><body><h1>LaserForge</h1>
<div id="login"><p>PIN</p><input id="pin" inputmode="numeric"> <button onclick="savePin()">OK</button></div>
<div id="ui" style="display:none">
<div class="st" id="st">…</div>
<div class="g"><button class="go" onclick="act('start')">▶ Avvia</button><button class="warn" onclick="act('pause')">❚❚ Pausa</button><button onclick="act('resume')">⏵ Riprendi</button>
<button class="stop" onclick="act('stop')">■ STOP</button><button onclick="act('frame')">⬚ Frame</button><button onclick="act('home')">⌂ Home</button></div>
<div class="g"><span></span><button onclick="jog(0,-1)">▲</button><span></span>
<button onclick="jog(-1,0)">◀</button><select id="step" style="font-size:17px;border-radius:10px"><option>0.1</option><option selected>1</option><option>10</option><option>50</option></select><button onclick="jog(1,0)">▶</button>
<span></span><button onclick="jog(0,1)">▼</button><span></span></div>
<div class="g"><button class="air" onclick="act('air_on')">Aria ON</button><span></span><button onclick="act('air_off')">Aria OFF</button></div>
</div>
<script>
let pin=localStorage.getItem('lfpin')||'';
function savePin(){pin=document.getElementById('pin').value;localStorage.setItem('lfpin',pin);poll();}
async function call(path,method){const r=await fetch(path,{method:method||'GET',headers:{'X-PIN':pin}});if(r.status==403){document.getElementById('login').style.display='';document.getElementById('ui').style.display='none';throw 0;}return r.json();}
async function act(a){try{const r=await call('/api/'+a,'POST');if(r.message)alert(r.message);}catch(e){}}
async function jog(x,y){const s=parseFloat(document.getElementById('step').value);try{await call('/api/jog?x='+x*s+'&y='+y*s,'POST');}catch(e){}}
async function poll(){try{const s=await call('/api/status');document.getElementById('login').style.display='none';document.getElementById('ui').style.display='';
document.getElementById('st').innerHTML='Stato: <b>'+s.state+'</b><br>X '+s.x.toFixed(2)+'  Y '+s.y.toFixed(2)+(s.total?'<br>Avanzamento: '+s.sent+'/'+s.total:'');}catch(e){}}
if(pin)poll();setInterval(()=>{if(pin)poll()},1000);
</script></body></html>"""


class _Bridge(QObject):
    command = Signal(str, str)


class RemoteServer:
    def __init__(self, win):
        self.win = win
        self.httpd = None
        self.thread = None
        self.bridge = _Bridge()
        self.bridge.command.connect(self._run)

    @staticmethod
    def local_ip():
        try:
            s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            s.connect(("10.255.255.255", 1))
            ip = s.getsockname()[0]
            s.close()
            return ip
        except OSError:
            return "127.0.0.1"

    @property
    def running(self):
        return self.httpd is not None

    def start(self, port, pin, host="0.0.0.0"):
        self.stop()
        srv = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _send(self, code, body, ctype="application/json"):
                data = body.encode() if isinstance(body, str) else body
                self.send_response(code)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.wfile.write(data)

            def _authorised(self):
                return pin and self.headers.get("X-PIN", "") == str(pin)

            def do_GET(self):
                url = urlparse(self.path)
                if url.path == "/":
                    self._send(200, PAGE, "text/html; charset=utf-8")
                elif url.path == "/api/status":
                    if not self._authorised():
                        return self._send(403, '{"error":"pin"}')
                    x, y, _z, ms = srv.win.engine.position()
                    sent, total = srv.win.engine.progress()
                    self._send(200, json.dumps({"state": srv.win.engine.state() + (f" ({ms})" if ms and ms != "Unknown" else ""),
                                                "x": x, "y": y, "sent": sent, "total": total}))
                else:
                    self._send(404, '{"error":"not found"}')

            def do_POST(self):
                url = urlparse(self.path)
                if not self._authorised():
                    return self._send(403, '{"error":"pin"}')
                action = url.path.removeprefix("/api/")
                if action not in ("start", "pause", "resume", "stop", "frame", "home", "jog", "air_on", "air_off"):
                    return self._send(404, '{"error":"unknown"}')
                if action == "start" and srv.win.prefs.get("checklist_enabled"):
                    return self._send(200, json.dumps({"message": "Checklist attiva: avvia dal computer."}))
                srv.bridge.command.emit(action, json.dumps({k: v[0] for k, v in parse_qs(url.query).items()}))
                self._send(200, '{"ok":true}')

        self.httpd = ThreadingHTTPServer((host, int(port)), Handler)
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self.thread.start()
        return f"http://{self.local_ip()}:{self.httpd.server_address[1]}"

    def stop(self):
        if self.httpd is not None:
            self.httpd.shutdown()
            self.httpd.server_close()
            self.httpd = None

    # runs in the Qt main thread
    def _run(self, action, args):
        w = self.win
        a = json.loads(args or "{}")
        if not w.connected and action not in ("stop",):
            w.toast(w.L("Comando remoto ignorato: laser non connesso", "Remote command ignored: laser not connected"))
            return
        if action == "start":
            w.start_laser()
        elif action == "pause":
            w.engine.pause()
        elif action == "resume":
            w.engine.resume()
        elif action == "stop":
            w.stop_laser()
        elif action == "frame":
            w.frame_job()
        elif action == "home":
            w.engine.home()
        elif action == "jog":
            w.jog_delta(float(a.get("x", 0)), float(a.get("y", 0)))
        elif action.startswith("air_"):
            w.engine.air("on" if action == "air_on" else "off")
        w.status(w.L("Comando dal telefono: ", "Remote command: ") + action)

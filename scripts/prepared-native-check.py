#!/usr/bin/python3
"""Real optimized native input, real subscription AI, disposable files only."""
import hashlib,json,os,runpy,subprocess,sys,time
from pathlib import Path
base=runpy.run_path(str(Path(__file__).with_name('release-ui-check.py')))
Session,UI,wait_xvfb=base['Session'],base['UI'],base['wait_xvfb']
runtime=Path(sys.argv[1]);out=Path(sys.argv[2]);out.mkdir(parents=True,exist_ok=True)
display=':218'
xvfb=subprocess.Popen([str(runtime.parent/'runtime/usr/bin/Xvfb'),display,'-screen','0','1600x1200x24','-nolisten','tcp'],stdout=(out/'xvfb.log').open('w'),stderr=subprocess.STDOUT,env=dict(os.environ,LD_LIBRARY_PATH=str(runtime.parent/'runtime/usr/lib')))
s=None
try:
 wait_xvfb(xvfb,display)
 s=Session(runtime,out,display,live=True)
 ui=s.ui
 ui.capture('native-ready.png')
 cache=s.root/'declared-cache';cache.mkdir()
 (cache/'CACHEDIR.TAG').write_bytes(b'Signature: 8a477f597d28d172789f06886806bc55\n# Disposable synthetic fixture cache intent\n')
 automatic=cache/'synthetic-payload.bin';automatic.write_bytes(b'SYNTHETIC CACHE ONLY\x00\xff'*131072)
 ui.activate('s');ui.capture('native-scanning.png')
 ui.wait(lambda:'scanned' in ui.text(),'real scan')
 ui.capture('native-scan.png')
 held={str(p):p.read_bytes() for p in s.root.rglob('*') if p.is_file() and p != automatic}
 selected_path=automatic
 original=selected_path.read_bytes()
 preimage=s.parent/'cache-payload-before.bin';preimage.write_bytes(original)
 assert '1 cache-intent candidates' in ui.text(), 'Evidence classifier must identify automatic fixture candidate'
 ui.activate('c');ui.capture('native-consent.png');ui.activate('i');ui.capture('native-analyzing.png')
 def proposed():
  t=ui.text()
  if 'Provider invocation failed' in t or 'Provider unavailable' in t or 'Provider returned invalid' in t:
   (out/'native-live-error.txt').write_text(t);raise AssertionError('Real integrated provider failed; see native-live-error.txt')
  return 'AI proposal · claude' in t
 ui.wait(proposed,'real integrated subscription proposal',timeout=110)
 ui.capture('native-live-proposal.png');(out/'native-live-proposal.txt').write_text(ui.text())
 ui.activate('p');ui.wait(lambda:'Review cleanup' in ui.text(),'fresh review');ui.capture('native-review.png',modal=True)
 assert str(selected_path) in ui.text(), 'Review must show exact synthetic local mapping'
 ui.activate('t');ui.capture('native-cleaning.png');ui.wait(lambda:not selected_path.exists(),'actual Trash move');ui.wait(lambda:'Moved to Trash' in ui.text(),'done')
 ui.capture('native-done.png')
 trashed=list((s.trash/'files').iterdir());assert len(trashed)==1
 assert trashed[0].read_bytes()==original
 ui.activate('h');ui.activate('r');ui.wait(lambda:'An existing file will' in ui.text(),'restore review');ui.capture('native-restore-review.png',modal=True)
 occupant=b'disposable collision occupant';selected_path.write_bytes(occupant)
 ui.activate('y');ui.wait(lambda:'File exists' in ui.text() or 'already exists' in ui.text().lower(),'collision refusal');time.sleep(.5)
 assert selected_path.read_bytes()==occupant and trashed[0].read_bytes()==original
 ui.capture('native-collision.png',modal=True);(out/'native-collision.txt').write_text(ui.text())
 # Move the synthetic collision occupant aside, preserving it for evidence.
 selected_path.rename(s.parent/'collision-occupant-preserved.bin')
 ui.activate('y');ui.wait(lambda:selected_path.exists() and selected_path.read_bytes()==original,'exact byte restore');ui.capture('native-restored.png')
 assert all(Path(p).read_bytes()==data for p,data in held.items())
 result={'source':'optimized native production bridge; real authenticated subscription Claude; XTest UI consent and confirmation','fixture':str(s.parent),'selected_path':str(selected_path),'preimage':str(preimage),'collision_saved':str(s.parent/'collision-occupant-preserved.bin'),'held_sha256':{p:hashlib.sha256(data).hexdigest() for p,data in held.items()},'sha256':hashlib.sha256(original).hexdigest(),'byte_equality':True,'collision_preserved':True,'runtime_capabilities':'Production parser required tools=[], mcp_servers=[], plugins=[] before accepting live result','automatic_candidate':True,'manual_file_confirmation':False,'held_files_byte_equality':True,'owner_scan':False,'purge':False}
 (out/'native-live-result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
finally:
 if s:s.close()
 xvfb.terminate();xvfb.wait(timeout=5)

#!/usr/bin/python3
"""Independent post-harness reads: synthetic bytes, runtime and packaged assets."""
import hashlib,io,json,subprocess,sys,tarfile
from pathlib import Path
out=Path(sys.argv[1]);runtime=Path(sys.argv[2]).resolve()
data=json.loads((out/('native-final-live/native-live-result.json' if (out/'native-final-live/native-live-result.json').exists() else 'native-live/native-live-result.json')).read_text())
fixture=Path(data['fixture']).resolve()
assert fixture.name.startswith('mem-inspector-release-check-')
restored,preimage=Path(data['selected_path']),Path(data['preimage'])
assert restored.resolve().is_relative_to(fixture) and preimage.resolve().is_relative_to(fixture)
assert restored.read_bytes()==preimage.read_bytes()
assert hashlib.sha256(restored.read_bytes()).hexdigest()==data['sha256']
assert Path(data['collision_saved']).read_bytes()==b'disposable collision occupant'
for name,digest in data['held_sha256'].items():
 path=Path(name);assert path.resolve().is_relative_to(fixture)
 assert hashlib.sha256(path.read_bytes()).hexdigest()==digest
assert not list((fixture/'Trash/files').iterdir())
with (out/'runtime-integrity.log').open('w') as log:
 subprocess.run(['sha256sum','-c','assets.sha256'],cwd=runtime,stdout=log,stderr=subprocess.STDOUT,check=True)
binary=(runtime/'bin/mem-inspector-desktop').read_bytes()
assert binary==Path('target/release/mem-inspector-desktop').read_bytes()
launch=json.loads((out/'home-final-launch/delivery-normal-launch.json').read_text())
assert launch['binary_sha256']==hashlib.sha256(binary).hexdigest()
assert launch['all_tmp_hidden_except_x11_socket'] and launch['no_scan_no_inference']
assets={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in Path('dist/ui/assets').iterdir()}
assert all(name.encode() in binary for name in assets), 'Final frontend asset keys must be embedded'
deb=Path('target/release/bundle/deb/Mem Inspector_0.2.0_amd64.deb')
member=next(n for n in subprocess.check_output(['ar','t',str(deb)],text=True).splitlines() if n.startswith('data.tar'))
with tarfile.open(fileobj=io.BytesIO(subprocess.check_output(['ar','p',str(deb),member]))) as archive:
 packed=archive.extractfile('usr/bin/mem-inspector-desktop').read()
assert packed.replace(b'__TAURI_BUNDLE_TYPE_VAR_DEB',b'__TAURI_BUNDLE_TYPE_VAR_UNK')==binary
assert (Path.home()/'.local/share/mem-inspector/current').resolve().name=='t141-d949c6f0e7dfcb5d28ac4882a89259d184f23be572927c740c9e987a1910623e'
result={'independent_byte_equality':True,'held_files_unchanged':True,'collision_occupant_preserved':True,'trash_files_empty':True,'runtime_integrity':True,'current_unchanged':True,'binary_sha256':hashlib.sha256(binary).hexdigest(),'embedded_frontend_assets':assets,'deb_matches_except_tauri_bundle_marker':True}
(out/'independent-verification.json').write_text(json.dumps(result,indent=2));print(json.dumps(result,indent=2))

#!/usr/bin/env python3
"""Publish only sanitized observations, never firmware profiles or heap padding."""
import argparse
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path

ROOT=Path(__file__).resolve().parent

def digest(b):
    return hashlib.sha256(b).hexdigest()


def main():
    ap=argparse.ArgumentParser()
    ap.add_argument('--capture',type=Path,required=True)
    ap.add_argument('--live',type=Path,required=True)
    ap.add_argument('--runner-commit',required=True)
    ap.add_argument('--runner',type=Path,required=True)
    ap.add_argument('--output',type=Path,required=True)
    a=ap.parse_args()
    raw=json.loads((a.capture/'capture.json').read_text())
    manifest=json.loads((ROOT/'cases.json').read_text())
    cases={c['id']:c for c in manifest}
    assert len(raw['cases'])==sum(c['firmware'] for c in manifest)
    published=[]
    for c in raw['cases']:
        fixture=cases[c['id']]
        assert c['input_sha256']==fixture['sha256']
        assert c['report']['status']=='passed',c['id']
        program=c['report']['programs'][0]
        source=(ROOT/fixture['source_file']).read_bytes().decode('utf-8')
        expected=source if fixture['container'] else source.replace('\r\n','\n').replace('\r','\n')
        expected_hash=digest(expected.encode())
        sessions=[]
        for label,saved in zip(['session1','session2'],c['saves']):
            observed=program[label]
            states={k:observed[k] for k in ['recognized_by_firmware','visible_in_catalog','opened_in_editor','source_verified']}
            assert all(s['ok'] for s in states.values()),(c['id'],label)
            assert saved['source_sha256']==expected_hash,(c['id'],label)
            sessions.append(dict(label=label,states=states,saved_sha256=saved['sha256'],saved_size=saved['size'],stored_source_sha256=saved['source_sha256'],firmware_exports=observed['firmware_exports']))
        published.append(dict(id=c['id'],input_sha256=c['input_sha256'],imported_sha256=c['imported_sha256'],original_source_sha256=digest(source.encode()),stored_source_sha256=expected_hash,sessions=sessions))
    editor=json.loads((a.capture/'editor-modification/capture.json').read_text())
    assert len(editor)==3
    assert all(e['source']=='EXPORT EDITTEST()\nBEGIN\n RETURN 2;\nEND;\n' for e in editor)
    live=json.loads(a.live.read_text())
    # Earlier probe versions attached an after-fallback frame observation to
    # the managed command. Do not attribute the UI fallback's pixels to it.
    live.get('same_session_assignment',{}).pop('catalog_changed',None)
    live.get('same_session_assignment',{}).pop('catalog_events',None)
    ui=live['ui_assignment']
    assert ui['created_file'] and ui['catalog_changed'] and ui['reopened_in_editor'] and ui['reopened_source_exact']
    actual=(a.live.parent/'reopened-source.txt').read_text().rsplit(' chars:\n',1)[-1]
    assert actual=='EXPORT NEWLIVE() BEGIN RETURN 3; END;\n'
    ui['reopened_source_sha256']=digest(actual[:-1].encode())
    assert live['host_live_install']['identical_catalog_rows']
    assert live['new_name_assignment']['execution_status']=='completed'
    for e in live.values():
        if isinstance(e,dict):e.pop('work_dir',None)
    result=dict(schema_version=1,provenance='local-firmware',captured_at_utc=datetime.now(timezone.utc).isoformat(),
        runner_commit=a.runner_commit,runner_sha256=digest(a.runner.read_bytes()),
        firmware_version=raw['firmware_version'],firmware_sha256=raw['firmware_sha256'],os_rom_sha256=raw['os_rom_sha256'],
        factory_image_sha256='c341d7a6450cdfb050665fea65eedf270c8c09e2fbe3c275b3d99300beb7efba',
        cases=published,editor_modification=[{k:e[k] for k in ['boot','saved_sha256','source_sha256']} for e in editor],live_import=live)
    a.output.parent.mkdir(parents=True,exist_ok=True)
    a.output.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
    print(f'Published {len(published)} genuine firmware cases, three editor boots, and separate live-import observations')


if __name__=='__main__':main()

#!/usr/bin/env python3
"""Assert a genuine GUI firmware Copy reached the operating-system clipboard."""
import json
import pathlib
import sys


def verify(path):
    data = json.loads(pathlib.Path(path).read_text())
    states = {s['label']: s['state'] for s in data['snapshots']}
    copied = states['firmware_copied']
    pasted = states['desktop_paste']
    assert not data['wait_timeouts'], data['wait_timeouts']
    assert copied['firmware_clipboard_available'] is True
    assert copied['firmware_alive'] is True
    assert copied['firmware_status'] == 'Copied firmware selection to desktop clipboard'
    # The source used physical calculator keys. This Ctrl+V goes through egui's
    # actual OS clipboard integration, without a Paste(text) automation step.
    assert pasted['native_session']['home_input'] == '246813579', pasted['native_session']
    assert pasted['native_session']['owner'] != 'Firmware'
    assert not any(s.get('step') == 'paste' for s in json.loads(
        pathlib.Path(__file__).with_name('scripts').joinpath('script_firmware_clipboard.json').read_text()
    )['steps'])
    print('7 firmware/OS clipboard assertions passed')


if __name__ == '__main__':
    verify(sys.argv[1])

#!/usr/bin/env python3
"""Verify literal editor delivery in a disposable installed-G1 profile.

The Program Editor receives and saves the replacement through actual UI input;
import creates only the initial zero-return program. Genuine execution must
return the edited result and exact source. Notes are created through the UI,
then read through the actual firmware Notes() API. No physical USB is opened.
"""
import argparse
import json
import pathlib
import shutil
import time

from verify_desktop_typing import png
from verify_firmware_clipboard import ClipboardSession, tap


def result(session, request, source):
    session.send('run', request=request, source=source, entry=None, arguments='')
    event = session.wait(lambda e: e.get('event') == 'result' and e.get('request') == request, 90)
    assert event['success'], event
    assert event['check']['accepted'], event
    return event


def clear_home(session):
    session.keys([0xe047, 0x8b, 0x01])
    session.settled()


def copy_current(session):
    generation = session.clipboard_generation
    session.keys([0x8b, 0xb4])
    session.settled()
    tap(session, 185, 230)
    tap(session, 294, 230)
    event = session.wait(lambda e: e.get('event') == 'clipboard'
                         and e['generation'] > generation, 15)
    session.keys([0x01, 0x01])
    session.settled()
    return event


def math_and_input(session, out):
    reports = []
    clear_home(session)
    session.keys([0x47])  # Genuine SIN template, caret starts inside it.
    session.settled()
    session.send('text', text='30', paste=False)
    png(out / 'sine-template.png', session.settled())
    copied = copy_current(session)
    assert copied['text'] == 'SIN(30)', copied
    reports.append(dict(case='math_template_digits', **copied))
    clear_home(session)
    session.send('text', text='"prefix"', paste=False)
    session.settled()
    session.keys([0x02])  # Move the actual calculator caret inside the string.
    session.settled()
    session.send('text', text='ⅈ×😀', paste=False)
    png(out / 'quoted-unicode.png', session.settled())
    copied = copy_current(session)
    assert copied['text'] == '"prefixⅈ×😀"', copied
    reports.append(dict(case='quoted_math_caret_unicode', **copied))
    clear_home(session)
    source = ('EXPORT TEXTINPUT() BEGIN LOCAL n:=0; '
              'IF INPUT(n,"Number")==0 THEN RETURN -1; END; RETURN n*2; END;')
    session.send('run', request=903, source=source, entry=None, arguments='')
    session.wait(lambda e: e.get('event') == 'phase' and e.get('phase') == 'running', 90)
    # Execution phase precedes the actual dialog. Require a settled UI before
    # and after typing. Enter accepts this single-field firmware INPUT form;
    # wait for its result directly so an inspection cannot consume it first.
    time.sleep(1)
    png(out / 'input-before.png', session.settled())
    session.send('text', text='12.5', paste=False)
    png(out / 'input-typed.png', session.settled())
    session.keys([0x0d])
    executed = session.wait(lambda e: e.get('event') == 'result' and e.get('request') == 903, 90)
    assert executed['success'] and executed['message'] == 'G1 firmware returned 25', executed
    reports.append(dict(case='running_ppl_input', **executed))
    return reports


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['runner', 'firmware', 'init-data', 'out']:
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    parser.add_argument('--os-rom', type=pathlib.Path)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    shutil.copytree(args.init_data, args.out / 'profile')
    session = ClipboardSession(args)
    reports = []
    source = 'EXPORT TEXTPROBE()\nBEGIN\n RETURN 17; // πⅈ 😀\nEND;'
    note = 'literal πⅈ 😀\nsecond line);'
    try:
        session.wait(lambda e: e['event'] == 'lifecycle' and e['state'] == 'ready', 180)
        session.send('import', request=900, name='TEXTPROBE',
                     source='EXPORT TEXTPROBE() BEGIN RETURN 0; END;')
        imported = session.wait(lambda e: e.get('event') == 'result' and e.get('request') == 900, 90)
        assert imported['success'], imported
        session.keys([0x8b, 0x59])  # Program Catalog.
        session.settled()
        tap(session, 50, 134)  # New program follows the two session helpers.
        png(args.out / 'program-before.png', session.settled())
        session.keys([0x8b, 0xb4])
        session.settled()
        tap(session, 185, 230)  # All; replacement must honor guest selection.
        session.send('text', text=source, paste=True)
        png(args.out / 'program-pasted.png', session.settled())
        session.keys([0x8b, 0xb4])
        session.settled()
        tap(session, 185, 230)
        tap(session, 294, 230)  # Copy real editor contents.
        copied = session.wait(lambda e: e.get('event') == 'clipboard' and e['text'] == source, 15)
        # Generation1 came from desktop Paste's real clipboard setter. Copy
        # must produce another genuine commit; an echoed Paste is insufficient.
        assert copied['generation'] >= 2, copied
        reports.append(dict(case='program_copy', **copied))
        session.keys([0x01, 0x01, 0xe047])
        session.settled()
        executed = result(session, 901,
                          'EXPORT VERIFYTEXT() BEGIN RETURN {TEXTPROBE(),Programs("TEXTPROBE")}; END;')
        escaped = source.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n')
        assert executed['message'] == 'G1 firmware returned {17, "' + escaped + '"}', executed
        reports.append(dict(case='edited_program_execution', **executed))
        session.keys([0x8b, 0x30])  # Note Catalog.
        session.settled()
        tap(session, 80, 230)  # New.
        session.settled()
        # Ordinary desktop typing must work in the genuine modal name editor.
        session.send('text', text='TEXTNOTE', paste=False)
        session.settled()
        session.keys([0x0d])  # Commit name field.
        session.settled()
        tap(session, 294, 230)  # Confirm New Note.
        session.settled()
        session.send('text', text=note, paste=True)
        png(args.out / 'note-pasted.png', session.settled())
        session.keys([0x01, 0xe047])  # Save editor and return Home.
        session.settled()
        readback = result(session, 902, 'EXPORT VERIFYNOTE() BEGIN RETURN Notes("TEXTNOTE"); END;')
        assert readback['message'] == 'G1 firmware returned ' + note, readback
        reports.append(dict(case='note_literal_readback', **readback))
        reports.extend(math_and_input(session, args.out))
        assert not any(e.get('event') in ('error', 'fault') for e in session.observed_events)
    except Exception:
        if session.frame is not None:
            png(args.out / 'failure.png', session.frame)
        raise
    finally:
        session.close()
        (args.out / 'events.json').write_text(json.dumps(session.observed_events, indent=2) + '\n')
        (args.out / 'report.json').write_text(json.dumps(reports, indent=2) + '\n')
    print(json.dumps(reports, indent=2))


if __name__ == '__main__':
    main()

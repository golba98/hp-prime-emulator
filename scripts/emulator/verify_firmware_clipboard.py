#!/usr/bin/env python3
"""Exercise genuine G1 Copy commits on a disposable local firmware profile.

The first source is entered solely with calculator keys. Clipboard text must
come from the guest, not a desktop typing cache or a screenshot transcription.
No physical calculator or existing writable user profile is opened.
"""
import argparse
import json
import pathlib
import shutil
import time
from verify_desktop_typing import Session, png


class ClipboardSession(Session):
    def __init__(self, args):
        self.observed_events = []
        super().__init__(args)

    def wait(self, predicate, timeout=90):
        def observe(event):
            if len(self.observed_events) < 5000:
                self.observed_events.append(event)
            return predicate(event)
        return super().wait(observe, timeout)


def tap(session, x, y):
    session.send('touch', status=1, x=x, y=y)
    for _ in range(18):
        time.sleep(.016)
        session.send('touch', status=2, x=x, y=y)
    session.send('touch', status=8, x=x, y=y)
    time.sleep(.7)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['runner', 'firmware', 'init-data', 'out']:
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    parser.add_argument('--os-rom', type=pathlib.Path)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    shutil.copytree(args.init_data, args.out / 'profile')
    session = ClipboardSession(args)
    results = []
    generation = 0
    try:
        session.wait(lambda e: e['event'] == 'lifecycle' and e['state'] == 'ready', 180)
        # Physical 2,4,6,8,1,3,5,7,9; no Command::Text is sent.
        session.keys([0x5a, 0x55, 0x57, 0x52, 0x59, 0x33, 0x56, 0x51, 0x53])
        session.settled()
        session.keys([0x8b, 0xb4])
        session.settled()
        tap(session, 185, 230)  # All
        png(args.out / 'selected.png', session.settled())
        for index in range(2):
            if index:
                time.sleep(2)  # Let the firmware's clipboard notification finish.
                # Copy may already have closed selection mode. Reset Home and
                # enter the same source with physical keys again; an extra Esc
                # must not accidentally clear the selection under test.
                session.keys([0xe047, 0x8b, 0x01])
                session.settled()
                session.keys([0x5a, 0x55, 0x57, 0x52, 0x59, 0x33, 0x56, 0x51, 0x53])
                session.settled()
                session.keys([0x8b, 0xb4])
                session.settled()
                tap(session, 185, 230)
                png(args.out / 'repeat-selected.png', session.settled())
            tap(session, 294, 230)  # Copy, including repeated Copy of same text.
            event = session.wait(lambda e: e['event'] == 'clipboard', 10)
            assert event['text'] == '246813579', event
            assert event['generation'] > generation, event
            generation = event['generation']
            png(args.out / f'copied-{index}.png', session.settled())
            results.append(dict(case='calculator_keys' if index == 0 else 'repeat_copy', **event))
        time.sleep(2)
        session.keys([0x01, 0x8b, 0x01])  # Exit selection, then Clear.
        session.settled()
        session.keys([0x53, 0x51, 0x56, 0x33, 0x59, 0x52, 0x57, 0x55, 0x5a])
        session.settled()
        session.keys([0x8b, 0xb4])
        session.settled()
        tap(session, 80, 230)  # End at the end of the entered source.
        session.keys([0x02, 0x02, 0x02])
        session.settled()
        tap(session, 25, 230)  # Begin three positions to the left.
        png(args.out / 'partial-selection.png', session.settled())
        tap(session, 294, 230)
        event = session.wait(lambda e: e['event'] == 'clipboard', 10)
        assert event['text'] == '642', event
        assert event['generation'] > generation, event
        generation = event['generation']
        results.append(dict(case='partial_selection', **event))
        time.sleep(2)
        session.keys([0x01])
        session.settled()
        # Import a new program into this disposable calculator, then copy its
        # editor contents. The expected text is compared exactly, including
        # guest-preserved line endings and mathematical Unicode characters.
        source = 'EXPORT CLIPTEXT()\nBEGIN\n RETURN "π×ⅈ";\nEND;'
        session.send('import', request=900, name='CLIPTEXT', source=source)
        imported = session.wait(lambda e: e['event'] == 'result' and e['request'] == 900, 90)
        assert imported['success'], imported
        session.keys([0x8b, 0x59])  # Program Catalog.
        png(args.out / 'program-catalog.png', session.settled())
        tap(session, 50, 134)  # The newly appended CLIPTEXT, after session helpers.
        png(args.out / 'program-editor.png', session.settled())
        for case, x in [('program_unicode_multiline', 294), ('program_cut', 240)]:
            if case == 'program_cut':
                # Reopen the same unmodified program independently of whether
                # the preceding Copy notification dismissed selection mode.
                session.keys([0x01, 0x01, 0xe047])
                session.settled()
                session.keys([0x8b, 0x59])
                session.settled()
                tap(session, 50, 50)  # Most recently used CLIPTEXT.
                session.settled()
            session.keys([0x8b])
            session.settled()
            session.keys([0xb4])
            session.settled()
            tap(session, 185, 230)
            tap(session, x, 230)
            event = session.wait(lambda e: e['event'] == 'clipboard', 10)
            assert event['text'] == source, event
            assert event['generation'] > generation, event
            generation = event['generation']
            results.append(dict(case=case, **event))
            png(args.out / (case + '.png'), session.settled())
            time.sleep(2)
    except Exception:
        if session.frame is not None:
            png(args.out / 'failure.png', session.frame)
        raise
    finally:
        session.close()
        (args.out / 'events.json').write_text(json.dumps(session.observed_events, indent=2) + '\n')
        (args.out / 'report.json').write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()

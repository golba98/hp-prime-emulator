import sys
path = sys.argv[1]
with open(path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

NL = chr(10)

# Bug 2: line 804 (0-indexed 803)
assert 'if tok == cf_up:' in lines[803], f'Bug2: {repr(lines[803])}'
lines[803] = '                            if tok == cf_up and curr_ln != fn_start_ln:' + NL
print('Bug 2 fixed')

# Bug 1: insert block_stack.append after line 695 (0-indexed 694)
assert 'if m_elseif:' in lines[694], f'Bug1: {repr(lines[694])}'
lines.insert(695, '                        # ELSE IF introduces a new nested IF block needing its own END' + NL)
lines.insert(696, "                        block_stack.append(('IF', curr_ln))" + NL)
print('Bug 1 fixed')

# Bug 3: reorder local tracking before skip (lines offset +2 from Bug1)
base = 782 + 2
assert lines[base].strip() == '', f'Bug3 blank: {repr(lines[base])}'
assert 'Skip structural keywords' in lines[base+1], f'Bug3: {repr(lines[base+1])}'
new_block = [
    '                    # Track local usage FIRST, before any skip/continue' + NL,
    '                    if tok in curr_locals or tok in used_locals_in_fn:' + NL,
    '                        used_locals_in_fn.add(tok)' + NL,
    NL,
    '                    # Skip structural keywords' + NL,
    "                    if tok in _STRUCTURAL or tok in BUILTINS or tok == 'LOCAL' or tok == 'EXPORT' or tok == 'PROCEDURE' or tok == 'BEGIN' or tok == 'END':" + NL,
    '                        continue' + NL,
    NL,
    '                    # Skip reserved globals (A-Z, G0-G9, etc.) - legacy PPL uses them' + NL,
    '                    if tok in _RESERVED_GLOBALS:' + NL,
    '                        continue' + NL,
    NL,
]
lines[base:base+14] = new_block
print('Bug 3 fixed')

# Bug 4: lines 412-413 (0-indexed)
assert 'simple split' in lines[412], f'Bug4: {repr(lines[412])}'
lines[412] = "            lhs_part = m_local.group(1).split(':=')[0]" + NL
assert 'm_local.group(1)' in lines[413], f'Bug4 forloop: {repr(lines[413])}'
lines[413] = "            for m_var in re.finditer(r'\\b([A-Za-z_]\\w*)\\b', lhs_part):" + NL
print('Bug 4 fixed')

# Bug 5: lines 575-577 (0-indexed)
assert '0-indexing check' in lines[575], f'Bug5: {repr(lines[575])}'
lines[575] = '            # 0-indexing check - skip built-in calls like WAIT(0)' + NL
lines[576] = "            for m_zero in re.finditer(r'\\b([A-Za-z_]\\w*)\\s*[\\(\\[\\s*0\\s*[\\)\\]', safe):" + NL
lines[577] = '                if m_zero.group(1).upper() not in BUILTINS:' + NL
lines.insert(578, '                    warn(curr_ln, "HP Prime arrays and lists are 1-indexed. Indexing with 0 will cause a runtime error.", display)' + NL)
lines.insert(579, '                    break' + NL)
del lines[580]
print('Bug 5 fixed')

with open(path, 'w', encoding='utf-8') as f:
    f.writelines(lines)
print('All 5 bugs patched successfully.')

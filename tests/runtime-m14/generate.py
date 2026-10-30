#!/usr/bin/env python3
"""Writes the M14 maths cases (programs/ + cases/) for `prime-emulator
app-suite --suite tests/runtime-m14`. Expected values are the mathematical
results, written before any firmware run; a mismatch is analysed, never
copied over from the firmware's output. Re-run after editing CASES."""
import hashlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
# (id, category, entry, expression returned by the program, expected display)
CASES = [
    ("cas-factor", "cas", "M14FACT", 'STRING(CAS("factor(x^2-1)"))', "(x-1)*(x+1)"),
    # The firmware writes minus as U+2212 (also in matrix-inverse).
    ("cas-solve", "cas", "M14SOLVE", 'STRING(CAS("solve(x^2-4=0,x)"))', "{\u22122,2}"),
    # STRING() of a CAS result in a Home program names functions in Home form.
    ("cas-diff", "cas", "M14DIFF", 'STRING(CAS("diff(sin(x),x)"))', "COS(x)"),
    ("cas-int", "cas", "M14INT", 'STRING(CAS("int(2*x,x)"))', "x^2"),
    ("cas-simplify", "cas", "M14SIMP", 'STRING(CAS("simplify((x^2-1)/(x-1))"))', "x+1"),
    ("cas-expand", "cas", "M14EXPD", 'STRING(CAS("expand((x+1)^2)"))', "x^2+2*x+1"),
    ("cas-exact-fraction", "cas", "M14FRAC", 'STRING(CAS("1/3+1/6"))', "1/2"),
    ("cas-exact-pi", "cas", "M14PI", 'STRING(CAS("3*pi-pi"))', "2*π"),
    ("cas-exact-trig", "cas", "M14TRIG", 'STRING(CAS("sin(pi/6)"))', "1/2"),
    ("cas-limit", "cas", "M14LIM", 'STRING(CAS("limit(sin(x)/x,x,0)"))', "1"),
    ("cas-symbolic", "cas", "M14SUBS", 'STRING(CAS("subst(a*x+b,x=2)"))', "2*a+b"),
    # The firmware's imaginary unit is U+E003 (a private-use glyph).
    ("complex-multiply", "complex", "M14CMUL", "STRING((1+2*i)*(3-i))", "5+5*\ue003"),
    ("complex-abs", "complex", "M14CABS", "ABS(3+4*i)", "5"),
    ("matrix-multiply", "matrix", "M14MMUL", "STRING([[1,2],[3,4]]*[[0,1],[1,0]])", "[[2,1],[4,3]]"),
    ("matrix-det", "matrix", "M14MDET", "DET([[1,2],[3,4]])", "-2"),
    ("matrix-inverse", "matrix", "M14MINV", "STRING([[1,2],[3,4]]^(-1))", "[[\u22122,1],[1.5,\u22120.5]]"),
    ("vector-cross", "vector", "M14CROS", "STRING(CROSS([1,0,0],[0,1,0]))", "[0,0,1]"),
    ("list-nested", "lists", "M14NEST", "{{1,{2,3}},4}", "{{1, {2, 3}}, 4}"),
    ("real-sqrt2", "real", "M14SQR2", "2^0.5", "1.41421356237"),
    ("real-third", "real", "M14THRD", "1/3", "0.333333333333"),
    ("home-fnroot", "solve", "M14ROOT", "FNROOT(X^2-4,X,1)", "2"),
    ("prob-comb", "probability", "M14COMB", "COMB(5,2)", "10"),
    # A decoded real is displayed by the host: 0.5.
    ("prob-normal", "probability", "M14NORM", "NORMALD_CDF(0)", "0.5"),
]
XFAIL = {
    "cas-int": "PPL CAS() returns 2*x^2/2; the CAS view returns x^2 (M14 m14_cas). Compared with the physical G1 in M15",
    "cas-exact-pi": "PPL CAS() returns 3*\u03c0-\u03c0 unsimplified; the CAS view returns 2*\u03c0. M15 compares with the G1",
    "cas-symbolic": "PPL CAS() returns a*2+b unsimplified; M15 compares with the G1",
    "real-third": "Returned as 1/3, while the physical G1 evidence (g1_smoke) shows 0.333333333333. Investigated in M15",
}
STATE = ("fresh boot; factory A drive + program-free initialized C: (init-data); "
         "program imported as C:\\DATA\\<NAME>.hpprgm")


def main():
    (HERE / "programs").mkdir(exist_ok=True)
    (HERE / "cases").mkdir(exist_ok=True)
    (HERE / "reference").mkdir(exist_ok=True)
    for cid, category, entry, expr, expected in CASES:
        body = expr if "RETURN" in expr else f"RETURN {expr}"
        source = f"EXPORT {entry}()\nBEGIN\n {body};\nEND;\n"
        prog = HERE / "programs" / f"{entry}.ppl"
        prog.write_text(source, encoding="utf-8")
        sha = hashlib.sha256(source.encode()).hexdigest()
        (HERE / "cases" / f"{cid}.toml").write_text(
            f'id = "{cid}"\ncategory = "{category}"\n'
            f"description = 'M14: {expr} through the firmware runtime'\n"
            f'program = "../programs/{entry}.ppl"\nsource_sha256 = "{sha}"\n'
            f'entry = "{entry}"\ninitial_state = \'{STATE}\'\n\n'
            + (f'expected_failure = "{XFAIL[cid]}"\n' if cid in XFAIL else "")
            + f'\n[expect]\noutcome = "completed"\nreturn_display = "{expected}"\n',
            encoding="utf-8")
    print(f"{len(CASES)} cases")


if __name__ == "__main__":
    main()

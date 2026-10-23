import contextlib
import io
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from src.ppl_emulator.runtime.engine import HPPrimeRuntime  # pyre-ignore
from src.ppl_emulator.transpiler import transpile  # pyre-ignore


def run_ppl(code, out_png=None):
    if out_png is None:
        out_png = os.path.join(tempfile.gettempdir(), "_ppl_runtime_parity.png")

    py_code = transpile(code, out_path=out_png)
    buf = io.StringIO()
    ns = {"__name__": "__main__", "__file__": "<runtime_parity>"}

    with contextlib.redirect_stdout(buf):
        exec(compile(py_code, "<runtime_parity>", "exec"), ns)

    return buf.getvalue(), ns.get("_rt"), py_code


def printed_lines(stdout_text):
    ignored_prefixes = ("pygame ", "Hello from the pygame community.")
    return [
        line.strip()
        for line in stdout_text.splitlines()
        if line.strip() and not line.strip().startswith(ignored_prefixes)
    ]


def test_binary_conversion_round_trip():
    code = """
    EXPORT T()
    BEGIN
      PRINT(STRING(R→B(10)));
      PRINT(B→R("#1010b"));
    END;
    """
    out, _, _ = run_ppl(code)
    assert printed_lines(out) == ["#1010b", "10"]


def test_grob_constructor_can_be_blitted():
    code = """
    EXPORT T()
    BEGIN
      RECT();
      LOCAL G := GROB(20, 20, #FF0000h);
      BLIT_P(G, 0, 0);
    END;
    """
    _, rt, _ = run_ppl(code)
    assert rt.img.getpixel((5, 5)) == (255, 0, 0)  # pyre-ignore


def test_fillpoly_p_fills_pixels():
    code = """
    EXPORT T()
    BEGIN
      RECT();
      FILLPOLY_P({10,10,20,10,15,20}, #00FF00h);
    END;
    """
    _, rt, _ = run_ppl(code)
    assert rt.img.getpixel((15, 15)) == (0, 255, 0)  # pyre-ignore


def test_textout_can_draw_to_target_grob():
    code = """
    EXPORT T()
    BEGIN
      RECT();
      DIMGROB_P(G1, 40, 20, #FFFFFFh);
      TEXTOUT_P("X", G1, 0, 0, 1, #000000h);
      BLIT_P(G1, 0, 0);
    END;
    """
    _, rt, _ = run_ppl(code)
    found_dark = False
    for y in range(0, 16):
        for x in range(0, 16):
            px = rt.img.getpixel((x, y))  # pyre-ignore
            if all(channel < 128 for channel in px[:3]):
                found_dark = True
                break
        if found_dark:
            break
    assert found_dark


def test_textout_p_returns_rendered_width():
    code = """
    EXPORT T()
    BEGIN
      DIMGROB_P(G1, 40, 20, #FFFFFFh);
      PRINT(TEXTOUT_P("Hello", G1, 0, 0, 1, #000000h));
    END;
    """
    out, _, _ = run_ppl(code)
    values = printed_lines(out)
    assert len(values) == 1
    assert int(values[0]) > 0


def test_subgrob_assigns_into_target_grob():
    code = """
    EXPORT T()
    BEGIN
      RECT();
      DIMGROB_P(G1, 20, 20, #FF0000h);
      DIMGROB_P(G2, 20, 20, #FFFFFFh);
      SUBGROB(G1, 0, 0, 19, 19, G2);
      BLIT_P(G2, 0, 0);
    END;
    """
    _, rt, _ = run_ppl(code)
    assert rt.img.getpixel((5, 5)) == (255, 0, 0)  # pyre-ignore


def test_blit_p_supports_target_then_dest_rect_then_source():
    code = """
    EXPORT T()
    BEGIN
      RECT();
      DIMGROB_P(G1, 20, 20, #FF0000h);
      DIMGROB_P(G2, 40, 40, #FFFFFFh);
      BLIT_P(G2, 10, 10, 20, 20, G1, 0, 0, 20, 20);
      BLIT_P(G2, 0, 0);
    END;
    """
    _, rt, _ = run_ppl(code)
    assert rt.img.getpixel((15, 15)) == (255, 0, 0)  # pyre-ignore


def test_cas_dot_methods_keep_runtime_method_names():
    code = """
    EXPORT T()
    BEGIN
      PRINT(CAS.diff("X^2", "X"));
      PRINT(CAS.integrate("X^2", "X"));
      PRINT(CAS.simplify("(X+1)*(X-1)"));
    END;
    """
    out, _, py_code = run_ppl(code)
    assert "CAS.diff" in py_code
    assert "CAS.DIFF" not in py_code
    assert printed_lines(out) == ["2*X", "X^3/3", "X^2 - 1"]


def test_arc_p_three_arg_form_draws_node_outline():
    code = """
    EXPORT T()
    BEGIN
      RECT();
      ARC_P(20, 20, 10);
    END;
    """
    _, rt, _ = run_ppl(code)
    assert rt.img.getpixel((30, 20)) == (0, 0, 0)  # pyre-ignore


def test_runtime_keeps_captured_runs_headless(monkeypatch):
    class _CapturedStream:
        def isatty(self):
            return False

    monkeypatch.setattr(sys, "stdout", _CapturedStream())
    monkeypatch.setattr(sys, "stderr", _CapturedStream())
    rt = HPPrimeRuntime()
    try:
        assert not rt._pg_enabled
    finally:
        rt.close()


def test_app_framework_tracks_current_view_and_setup():
    code = """
    EXPORT T()
    BEGIN
      PRINT(START("Function"));
      PRINT(VIEW());
      SymbSetup("Grid", 1);
      PlotSetup("Axes", 0);
      NumSetup("Precision", 4);
      PRINT(SymbSetup("Grid"));
      Plot();
      PRINT(VIEW());
      PRINT(PlotSetup("Axes"));
      Num();
      PRINT(VIEW());
      PRINT(NumSetup("Precision"));
      VIEW("Custom");
      PRINT(VIEW());
      RESET();
      PRINT(VIEW());
      PRINT(SymbSetup("Grid"));
    END;
    """
    out, rt, _ = run_ppl(code)
    assert printed_lines(out) == [
        "Function",
        "Symb",
        "1",
        "Plot",
        "0",
        "Num",
        "4",
        "Custom",
        "Symb",
        "0",
    ]
    assert str(rt.GET_VAR("APROGRAM").value) == "Function"  # pyre-ignore


def test_num_keeps_numeric_conversion_when_called_with_argument():
    code = """
    EXPORT T()
    BEGIN
      PRINT(NUM("12.5"));
      Num();
      PRINT(VIEW());
    END;
    """
    out, _, _ = run_ppl(code)
    assert printed_lines(out) == ["12.5", "Num"]


def test_notes_afiles_and_grob_size_helpers_round_trip_values():
    code = """
    EXPORT T()
    BEGIN
      Notes("HELLO"):="abc";
      PRINT(Notes("HELLO"));
      DIMGROB_P(G1, 12, 7, #123456h);
      AFiles("PIC"):=G1;
      G2:=AFiles("PIC");
      PRINT(GROBW_P(G2));
      PRINT(GROBH_P(G2));
      DelAFiles("PIC");
      PRINT(SIZE(AFiles()));
    END;
    """
    out, _, _ = run_ppl(code)
    assert printed_lines(out) == ["abc", "12", "7", "0"]


def test_text_helpers_and_collection_helpers_match_prime_style_usage():
    code = """
    EXPORT T()
    BEGIN
      LOCAL s := "1>Hello";
      LOCAL items := {};
      items := APPEND(items, {TAIL(s), EXPR(HEAD(s))});
      PRINT(items(1,1));
      PRINT(items(1,2));
      PRINT(BITSL(3, 2));
      PRINT(BITSR(16, 2));
      TEXT_CLEAR();
      TEXT_AT(2, 3, "OK");
      LOCAL mat := MAKEMAT(7, 2, 3);
      LOCAL zeroMat := MAKEMAT(2, 2);
      PRINT(mat(2,3));
      PRINT(zeroMat(1,1));
    END;
    """
    out, rt, _ = run_ppl(code)
    assert printed_lines(out) == ["\u003eHello", "1", "12", "4", "7", "0"]
    assert len(rt._terminal_lines) >= 2
    assert rt._terminal_lines[1].startswith("  OK")


def test_rect_p_and_blit_p_accept_numeric_grob_slots():
    code = """
    EXPORT T()
    BEGIN
      RECT_P(1, 0, 0, 10, 10, #FF0000h, #FF0000h);
      BLIT_P(0, 1, 0, 0, 10, 10);
    END;
    """
    _, rt, _ = run_ppl(code)
    assert rt.img.getpixel((5, 5)) == (255, 0, 0)  # pyre-ignore

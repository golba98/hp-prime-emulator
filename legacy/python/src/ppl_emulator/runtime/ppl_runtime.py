import sympy
import re

def _ppl_to_sympy(s):
    """Bridge PPL syntax/variables to SymPy."""
    if isinstance(s, sympy.Basic):
        return s
    if not isinstance(s, str):
        return sympy.sympify(s)
    s = s.replace('^', '**')
    return sympy.sympify(s.lower())

def _sympy_to_ppl(expr):
    """Bridge SymPy result back to PPL format (uppercase X, ^ for power)."""
    return str(expr).replace('**', '^').upper()

def ppl_expr(s, x_val=None):
    """Evaluates a PPL expression string. Supports exact fractions and symbolic substitution."""
    try:
        expr = _ppl_to_sympy(str(s))
        if x_val is not None:
            val_sym = _ppl_to_sympy(x_val) if isinstance(x_val, str) else sympy.sympify(x_val)
            expr = expr.subs(sympy.Symbol('x'), val_sym)
        expr = sympy.simplify(expr)
        if expr.is_number:
            if expr.is_Integer:
                return int(expr)
            res_float = float(expr.evalf())
            if res_float.is_integer():
                return int(res_float)
            return res_float
        return _sympy_to_ppl(expr)
    except Exception as e:
        return f"Error evaluating expression: {e}"

class CAS:
    """Symbolic Math Engine for PPL."""
    def __init__(self, runtime=None):
        self._rt = runtime

    def __call__(self, s):
        return ppl_expr(s)

    def diff(self, expr_str, var="X"):
        try:
            return _sympy_to_ppl(sympy.diff(_ppl_to_sympy(str(expr_str)), sympy.Symbol(str(var).lower())))
        except Exception as ex:
            return f"Error in CAS.diff: {ex}"

    def integrate(self, expr_str, var="X"):
        try:
            return _sympy_to_ppl(sympy.integrate(_ppl_to_sympy(str(expr_str)), sympy.Symbol(str(var).lower())))
        except Exception as ex:
            return f"Error in CAS.integrate: {ex}"

    def simplify(self, expr_str):
        try:
            return _sympy_to_ppl(sympy.simplify(_ppl_to_sympy(str(expr_str))))
        except Exception as ex:
            return f"Error in CAS.simplify: {ex}"

    def solve(self, expr_str, var="X"):
        try:
            e = _ppl_to_sympy(str(expr_str))
            v = sympy.Symbol(str(var).lower())
            return [_sympy_to_ppl(s) for s in sympy.solve(e, v)]
        except Exception as ex:
            return f"Error in CAS.solve: {ex}"

    def zeros(self, expr_str, var="X"):
        """Find real zeros/roots of an expression (PPL ZEROS)."""
        try:
            e = _ppl_to_sympy(str(expr_str))
            v = sympy.Symbol(str(var).lower())
            result = []
            for s in sympy.solve(e, v):
                if s.is_real is False:
                    continue
                result.append(int(s) if (s.is_number and s.is_Integer) else (float(s.evalf()) if s.is_number else _sympy_to_ppl(s)))
            return result
        except Exception as ex:
            return f"Error in CAS.zeros: {ex}"

    def czeros(self, expr_str, var="X"):
        """Find all zeros including complex (PPL CZEROS)."""
        try:
            e = _ppl_to_sympy(str(expr_str))
            v = sympy.Symbol(str(var).lower())
            result = []
            for s in sympy.solve(e, v):
                if s.is_number:
                    c = complex(s.evalf())
                    result.append(c if c.imag != 0 else (int(s) if s.is_Integer else float(s.evalf())))
                else:
                    result.append(_sympy_to_ppl(s))
            return result
        except Exception as ex:
            return f"Error in CAS.czeros: {ex}"

    def factor(self, expr_str):
        try:
            return _sympy_to_ppl(sympy.factor(_ppl_to_sympy(str(expr_str))))
        except Exception as ex:
            return f"Error in CAS.factor: {ex}"

    def expand(self, expr_str):
        try:
            return _sympy_to_ppl(sympy.expand(_ppl_to_sympy(str(expr_str))))
        except Exception as ex:
            return f"Error in CAS.expand: {ex}"

    def partfrac(self, expr_str, var="X"):
        try:
            e = _ppl_to_sympy(str(expr_str))
            v = sympy.Symbol(str(var).lower())
            return _sympy_to_ppl(sympy.apart(e, v))
        except Exception as ex:
            return f"Error in CAS.partfrac: {ex}"

    def limit(self, expr_str, var="X", point="0", direction="+"):
        try:
            e = _ppl_to_sympy(str(expr_str))
            v = sympy.Symbol(str(var).lower())
            p = _ppl_to_sympy(str(point))
            res = sympy.limit(e, v, p, direction)
            if res.is_number:
                return int(res) if res.is_Integer else float(res.evalf())
            return _sympy_to_ppl(res)
        except Exception as ex:
            return f"Error in CAS.limit: {ex}"

    def series(self, expr_str, var="X", point="0", order=6):
        try:
            e = _ppl_to_sympy(str(expr_str))
            v = sympy.Symbol(str(var).lower())
            p = _ppl_to_sympy(str(point))
            res = sympy.series(e, v, p, int(order))
            return _sympy_to_ppl(res.removeO())
        except Exception as ex:
            return f"Error in CAS.series: {ex}"

    def taylor(self, *args, **kwargs):
        return self.series(*args, **kwargs)

    def __getattr__(self, name):
        lower_name = name.lower()
        if lower_name != name:
            try:
                return object.__getattribute__(self, lower_name)
            except AttributeError:
                pass
        def stub(*args, **kwargs):
            return f"CAS.{name}: not implemented"
        return stub


def DET(matrix):
    """Calculates the determinant of a PPL matrix (nested lists)."""
    try:
        m = sympy.Matrix(matrix)
        res = m.det()
        if res.is_number:
            return int(res) if res.is_Integer else float(res.evalf())
        return _sympy_to_ppl(res)
    except Exception as e:
        return f"Error in DET: {e}"

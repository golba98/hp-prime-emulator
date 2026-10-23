from src.ppl_emulator.runtime.resource_budget import get_active_budget


def _budget_sync():
    budget = get_active_budget()
    if budget is not None and getattr(budget, "active", False):
        budget.recalculate()


class PPLVar:
    """Boxed PPL variable — wraps any value so it can be passed by reference.

    All math, comparison, iteration, and indexing operations delegate to
    ``self.value``, so callers rarely need to unwrap it manually.
    """
    def __init__(self, value):
        self.value = value
    def __repr__(self):
        return str(self.value)
    def __str__(self):
        return str(self.value)
    # Support basic math to avoid crashes if dereferencing is missed
    def __add__(self, other): return self.value + other
    def __radd__(self, other): return other + self.value
    def __sub__(self, other): return self.value - other
    def __rsub__(self, other): return other - self.value
    def __mul__(self, other): return self.value * other
    def __rmul__(self, other): return other * self.value
    def __truediv__(self, other): return self.value / other
    def __rtruediv__(self, other): return other / self.value
    def __floordiv__(self, other): return self.value // other
    def __rfloordiv__(self, other): return other // self.value
    def __mod__(self, other): return self.value % other
    def __rmod__(self, other): return other % self.value
    def __pow__(self, other): return self.value ** other
    def __rpow__(self, other): return other ** self.value
    def __abs__(self): return abs(self.value)
    def __neg__(self): return -self.value
    def __pos__(self): return +self.value
    def __int__(self): return int(self.value)
    def __float__(self): return float(self.value)
    def __bool__(self): return bool(self.value)
    def __eq__(self, other): return self.value == (other.value if isinstance(other, PPLVar) else other)
    def __lt__(self, other): return self.value < (other.value if isinstance(other, PPLVar) else other)
    def __le__(self, other): return self.value <= (other.value if isinstance(other, PPLVar) else other)
    def __gt__(self, other): return self.value > (other.value if isinstance(other, PPLVar) else other)
    def __ge__(self, other): return self.value >= (other.value if isinstance(other, PPLVar) else other)

    # Delegate indexing and calling to the underlying value (e.g. for PPLList/PPLString)
    def __getitem__(self, i):
        # Discrepancy 4 fix: only suppress expected access errors, let logic errors bubble
        return self.value[i]

    def __setitem__(self, i, val):
        self.value[i] = val
        _budget_sync()

    def __call__(self, *args, **kwargs):
        # Allow boxed callables (e.g. a PPLList stored in a var) to be called directly.
        # If the underlying value isn't callable, return it as-is rather than crashing.
        try:
            return self.value(*args, **kwargs)
        except TypeError:
            return self.value

    def __iter__(self):
        try:
            return iter(self.value)
        except TypeError:
            return iter([self.value])

    def __len__(self):
        try:
            return len(self.value)
        except TypeError:
            return 0


class PPLList(list):
    """1-indexed list for PPL compatibility.

    Discrepancy 2 fix: __getitem__ and __setitem__ now internally convert
    1-based PPL indices to 0-based Python indices. The transpiler no longer
    subtracts 1 when generating bracket access from PPL paren-syntax, so
    PPLList is the single source of truth for index conversion.
    """

    def __call__(self, *args):
        # Delegate to __getitem__ which now handles 1-based conversion.
        if len(args) == 1:
            return self[int(args[0])]
        # Multi-dimensional: p(j, k) -> p[j][k], 1-based via __getitem__
        result = self
        for idx in args:
            result = result[int(idx)]
        return result

    def __getitem__(self, i):
        if isinstance(i, slice):
            # Slices arrive pre-adjusted from _ppl_to_py_slice (0-based bounds).
            return PPLList(super().__getitem__(i))
        # 1-based: subtract 1 here so callers pass PPL indices directly.
        try:
            return super().__getitem__(int(i) - 1)
        except IndexError:
            return 0
        except TypeError:
            return 0

    def __setitem__(self, i, val):
        if isinstance(i, slice):
            # Slices arrive pre-adjusted; pass through directly.
            super().__setitem__(i, val)
            _budget_sync()
            return
        # 1-based: subtract 1.
        idx = int(i) - 1
        try:
            super().__setitem__(idx, val)
            _budget_sync()
        except IndexError:
            # PPL automatically expands lists on out-of-bounds assignment.
            while len(self) <= idx:
                self.append(0)
            super().__setitem__(idx, val)
            _budget_sync()

    def __hash__(self):
        def make_hashable(obj):
            if isinstance(obj, (list, PPLList)):
                return tuple(make_hashable(x) for x in obj)
            return obj
        return hash(make_hashable(self))

    def __eq__(self, other: object) -> bool:
        try:
            return list(self) == list(other)  # pyre-ignore
        except TypeError:
            return False

    def sort_inplace(self, reverse=False):
        super().sort(reverse=reverse)
        _budget_sync()
        return self

    def insert(self, idx, val):
        # Already 1-based before this fix; kept as-is.
        super().insert(max(0, int(idx) - 1), val)
        _budget_sync()

    def pop(self, idx=None):
        if idx is None:
            try:
                value = super().pop()
                _budget_sync()
                return value
            except IndexError:
                return 0
        try:
            value = super().pop(int(idx) - 1)
            _budget_sync()
            return value
        except IndexError:
            return 0

    def index_of(self, val):
        try:
            return super().index(val) + 1
        except ValueError:
            return 0


class PPLMatrix:
    """Fixed-dimension mutable matrix for PPL compatibility.

    Discrepancy 1 fix: on real HP Prime hardware, matrix elements ARE mutable
    via index assignment (e.g. M(i,j) := val). Structural changes (append,
    extend, pop) are forbidden since they would alter the matrix dimensions.

    Uses 1-based indexing, consistent with PPLList.
    """

    def __init__(self, data):
        if isinstance(data, PPLMatrix):
            self._data = [list(row) for row in data._data]
        else:
            self._data = [list(row) for row in data]
        self._rows = len(self._data)
        self._cols = len(self._data[0]) if self._data else 0

    def __repr__(self):
        inner = ", ".join("[" + ", ".join(str(x) for x in row) + "]" for row in self._data)
        return f"PPLMatrix([{inner}])"

    def __str__(self):
        rows = ["[" + ", ".join(str(x) for x in row) + "]" for row in self._data]
        return "[" + ", ".join(rows) + "]"

    def __len__(self):
        return self._rows

    def __iter__(self):
        for row in self._data:
            yield PPLList(row)

    def __getitem__(self, key):
        if isinstance(key, tuple):
            r, c = key
            return self._data[int(r) - 1][int(c) - 1]
        # Single index: return the row as a PPLList (1-based row index).
        return PPLList(self._data[int(key) - 1])

    def __setitem__(self, key, value):
        if isinstance(key, tuple):
            r, c = key
            self._data[int(r) - 1][int(c) - 1] = value
            _budget_sync()
        else:
            # Assign an entire row; dimensions must stay the same.
            row_idx = int(key) - 1
            if isinstance(value, (list, PPLList)):
                if len(value) != self._cols:
                    raise ValueError(
                        f"Matrix dimension mismatch: expected {self._cols} columns, got {len(value)}"
                    )
                self._data[row_idx] = list(value)
                _budget_sync()
            else:
                raise TypeError("Matrix row must be a list")

    def __call__(self, *args):
        """Support M(r, c) syntax for 1-based element access."""
        if len(args) == 2:
            return self._data[int(args[0]) - 1][int(args[1]) - 1]
        if len(args) == 1:
            return PPLList(self._data[int(args[0]) - 1])
        raise IndexError(f"PPLMatrix requires 1 or 2 indices, got {len(args)}")

    def dim(self):
        return PPLList([self._rows, self._cols])

    # Forbid dimension-altering operations.
    def append(self, *args):
        raise TypeError("Cannot append to a PPLMatrix (would alter fixed dimensions)")

    def extend(self, *args):
        raise TypeError("Cannot extend a PPLMatrix (would alter fixed dimensions)")

    def pop(self, *args):
        raise TypeError("Cannot pop from a PPLMatrix (would alter fixed dimensions)")

    def sort(self, *args, **kwargs):
        raise TypeError("Cannot sort a PPLMatrix in-place; convert to a list first")

    def __eq__(self, other):
        if isinstance(other, PPLMatrix):
            return self._data == other._data
        return NotImplemented

    def __hash__(self):
        return hash(tuple(tuple(row) for row in self._data))

    def __mul__(self, other):
        """Matrix multiplication (dot product) or scalar multiplication."""
        # Scalar multiplication
        if isinstance(other, (int, float)):
            new_data = [[cell * other for cell in row] for row in self._data]
            return PPLMatrix(new_data)

        # Matrix multiplication (Dot Product)
        if isinstance(other, PPLMatrix):
            if self._cols != other._rows:
                raise ValueError(
                    f"Dimension mismatch: Cannot multiply ({self._rows}x{self._cols}) "
                    f"by ({other._rows}x{other._cols})"
                )

            # Standard dot product implementation
            result = [[0 for _ in range(other._cols)] for _ in range(self._rows)]
            for i in range(self._rows):
                for j in range(other._cols):
                    dot_product = 0
                    for k in range(self._cols):
                        dot_product += self._data[i][k] * other._data[k][j]
                    result[i][j] = dot_product
            return PPLMatrix(result)

        return NotImplemented

    def __rmul__(self, other):
        """Commutative scalar multiplication (scalar * matrix)."""
        return self.__mul__(other)


class CASMock:
    """Stub CAS object — used when SymPy is unavailable or a real CAS call is not needed.

    All methods return a harmless default (0.5 for scalars, empty list for collections).
    """

    def __call__(self, cmd):
        return 0.5

    def LambertW(self, z, k=0):
        return 0.5

    def solve(self, *args):
        return PPLList([])

    def __getattr__(self, name):
        return lambda *args, **kwargs: PPLList([])

class PPLString:
    """PPL-compatible string that supports implicit addition and 1-based indexing.

    Discrepancy 2 fix: __getitem__ and __setitem__ now use 1-based indexing
    internally, consistent with PPLList.
    """
    def __init__(self, data=""):
        if isinstance(data, (list, PPLList)):
            self.data = "".join(str(x) for x in data)
        elif isinstance(data, PPLString):
            self.data = data.data
        else:
            self.data = str(data)

    def __str__(self):
        return self.data

    def __repr__(self):
        return f'PPLString("{self.data}")'

    def __len__(self):
        return len(self.data)

    def __add__(self, other):
        return PPLString(self.data + str(other))

    def __radd__(self, other):
        return PPLString(str(other) + self.data)

    def __eq__(self, other):
        return self.data == str(other)

    def __lt__(self, other): return self.data <  str(other)
    def __le__(self, other): return self.data <= str(other)
    def __gt__(self, other): return self.data >  str(other)
    def __ge__(self, other): return self.data >= str(other)

    def __getitem__(self, i):
        if isinstance(i, slice):
            # Slices arrive pre-adjusted (0-based from _ppl_to_py_slice).
            return PPLString(self.data[i])
        # 1-based: subtract 1 internally.
        try:
            idx = int(i) - 1
            if 0 <= idx < len(self.data):
                return self.data[idx]
            return ""
        except (ValueError, TypeError):
            return ""

    def __setitem__(self, i, val):
        try:
            idx = int(i) - 1  # 1-based
            if idx < 0:
                return
            chars = list(self.data)
            while len(chars) <= idx:
                chars.append(" ")   # pad with spaces if index is beyond current length
            v_str = str(val)
            chars[idx] = v_str[0] if v_str else " "
            self.data = "".join(chars)
            _budget_sync()
        except (ValueError, TypeError):
            pass

    def __call__(self, *args):
        # s(4) -> char at 1-based index 4 — delegate to __getitem__
        if len(args) == 1:
            return self[args[0]]
        # s(1, 2) -> substring from 1-based index 1 to 2 (inclusive)
        if len(args) == 2:
            try:
                start = int(args[0]) - 1
                end = int(args[1])
                return PPLString(self.data[start:end])
            except (ValueError, TypeError):
                return PPLString("")
        return self

    def __iter__(self):
        return iter(self.data)

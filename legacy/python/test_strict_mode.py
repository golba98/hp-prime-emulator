from src.ppl_emulator.transpiler.core import transpile
import pytest

def test_lvalue_validation():
    code = """
EXPORT test()
BEGIN
  5 := A;
END;
"""
    with pytest.raises(SyntaxError, match="L-value must be a variable"):
        transpile(code)

def test_system_global_shadowing():
    code = """
EXPORT test()
BEGIN
  LOCAL Ans;
END;
"""
    with pytest.raises(SyntaxError, match="Cannot shadow system global 'Ans'"):
        transpile(code)

def test_strict_for_loop():
    code = """
EXPORT test()
BEGIN
  FOR I FROM 1 TO 10 BEGIN
    PRINT(I);
  END;
END;
"""
    with pytest.raises(SyntaxError, match="Expected 'DO' in FOR loop"):
        transpile(code)

def test_block_stack_integrity():
    code = """
EXPORT test()
BEGIN
  IF 1 THEN
    PRINT("Hello");
END;
"""
    with pytest.raises(SyntaxError, match="BEGIN block at line 3 was never closed"):
        transpile(code)

def test_unclosed_begin():
    code = """
EXPORT test()
BEGIN
  PRINT("Hello");
"""
    with pytest.raises(SyntaxError, match="BEGIN block at line 3 was never closed"):
        transpile(code)

if __name__ == "__main__":
    # Manually run tests if pytest is not available or just to see output
    try:
        test_lvalue_validation()
        print("test_lvalue_validation passed")
    except Exception as e:
        print(f"test_lvalue_validation failed: {e}")

    try:
        test_system_global_shadowing()
        print("test_system_global_shadowing passed")
    except Exception as e:
        print(f"test_system_global_shadowing failed: {e}")

    try:
        test_strict_for_loop()
        print("test_strict_for_loop passed")
    except Exception as e:
        print(f"test_strict_for_loop failed: {e}")

    try:
        test_block_stack_integrity()
        print("test_block_stack_integrity passed")
    except Exception as e:
        print(f"test_block_stack_integrity failed: {e}")

    try:
        test_unclosed_begin()
        print("test_unclosed_begin passed")
    except Exception as e:
        print(f"test_unclosed_begin failed: {e}")

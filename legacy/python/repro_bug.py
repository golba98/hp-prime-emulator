from src.ppl_emulator.runtime.engine import HPPrimeRuntime
from src.ppl_emulator.runtime.types import PPLList

def test_repro():
    rt = HPPrimeRuntime()
    # Test matrix literal assignment (triggers _coerce_list)
    try:
        m_val = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
        rt.SET_VAR('M', m_val, is_local=True)
        print("Matrix assigned successfully")
    except UnboundLocalError as e:
        print(f"Caught expected error: {e}")
        return

    # Test 2D indexing m(i, j)
    m = rt.GET_VAR('M').value
    print(f"Type of M: {type(m)}")
    print(f"M(1, 1) = {m(1, 1)}")
    print(f"M(1, 2) = {m(1, 2)}")
    print(f"M(2, 1) = {m(2, 1)}")
    
    # Expected output for the user's test: 1 2 3 4 5 6 7 8 9
    for i in range(1, 4):
        for j in range(1, 4):
            print(m(i, j))

if __name__ == "__main__":
    test_repro()

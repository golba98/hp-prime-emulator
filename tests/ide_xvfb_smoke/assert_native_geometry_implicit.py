"""Read four independently measured implicit Geometry constructions through PPL and Plot."""
import json
from pathlib import Path
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/ppl-ide-e2e")
for suffix in ("", "_reopen"):
    dump = json.loads((root / f"native_side_geometry_implicit{suffix}.json").read_text())
    assert not dump["wait_timeouts"], dump["wait_timeouts"]
    for item in dump["snapshots"]:
        state = item["state"]
        values = [row["value"]["Ok"] for row in
                  state["native_model_results"]["Geometry:plot"]["geometry"]["objects"]]
        assert values[0]["Line"] == {"a":{"x":-3,"y":0},"b":{"x":3,"y":0},"extent":"Segment"}
        assert values[1]["Parametric"]["variable"] == "u"
        assert len(values[2]["Objects"]) == len(values[3]["Lines"]) == 2
        if item["label"] == "ppl":
            outcome = state["native_session"]["last_outcome"]
            assert outcome["status"] == "success", outcome
            assert [v["value"] for v in outcome["typed_result"]["value"]] == ["0","2","2","2"]
print("Implicit segments, rational curves, disconnected branches and cold PPL reads passed")

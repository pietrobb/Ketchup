from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from ketchup_sdk import (  # noqa: E402
    Client,
    Manifest,
    EDIT_CAD_PROGRAM,
    QUERY_AGENT_STATE,
)


def main() -> None:
    target = int(sys.argv[1])
    value_mm = float(sys.argv[2])
    client = Client(
        Manifest(
            package="org.ketchup.dimension-pilot",
            version="1.0.0",
            principal_id=7001,
            capabilities=(QUERY_AGENT_STATE, EDIT_CAD_PROGRAM),
        )
    )
    client.start()
    state = client.query_agent_state()
    if "ketchup.state-view.agent.v2" not in state:
        raise RuntimeError("host did not return Agent StateView v2")
    accepted = client.apply_operations(
        [{"operation": "set_dimension", "feature_id": target, "value_mm": value_mm}]
    )
    if accepted != 1:
        raise RuntimeError("host did not accept the one set_dimension operation")
    client.finish()


if __name__ == "__main__":
    main()

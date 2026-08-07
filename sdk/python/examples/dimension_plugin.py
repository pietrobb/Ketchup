from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from ketchup_sdk import (  # noqa: E402
    Client,
    Manifest,
    QUERY_AGENT_STATE,
    SET_FEATURE_DIMENSION,
)


def main() -> None:
    target = int(sys.argv[1])
    value = sys.argv[2]
    client = Client(
        Manifest(
            package="org.ketchup.dimension-pilot",
            version="1.0.0",
            principal_id=7001,
            capabilities=(QUERY_AGENT_STATE, SET_FEATURE_DIMENSION),
        )
    )
    client.start()
    state = client.query_agent_state()
    if "ketchup.state-view.agent.v1" not in state:
        raise RuntimeError("host did not return Agent StateView v1")
    receipt = client.set_feature_dimension(target, value)
    if receipt.commands != 1 or receipt.writes != 1:
        raise RuntimeError("host Proposal exceeded the pilot write envelope")
    client.finish()


if __name__ == "__main__":
    main()

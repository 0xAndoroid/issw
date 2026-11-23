import json
import os
import re
import subprocess
from datetime import datetime
from urllib.parse import urlencode

BRANCH = "main"
SUITE_STEP = "Run cargo nextest run --cargo-quiet --no-tests=fail"
SKIP_STEP = "Nightly SHA already covered"


def api(path, **params):
    query = f"?{urlencode(params)}" if params else ""
    response = subprocess.check_output(
        ["gh", "api", f"{path}{query}"], text=True, timeout=20
    )
    return json.loads(response)


def completed_nightly(run, actions, before):
    run_path = f"{actions}/runs/{run['id']}"
    jobs = api(f"{run_path}/jobs", filter="all", per_page=100)["jobs"]
    if not isinstance(jobs, list):
        raise ValueError("Invalid nightly jobs response")
    for job in jobs:
        if (
            job["run_id"] != run["id"]
            or job["head_sha"] != run["head_sha"]
            or job["status"] != "completed"
            or job["conclusion"] not in {"success", "failure"}
            or datetime.fromisoformat(job["completed_at"]) >= before
        ):
            continue
        completed_steps = {
            step["name"]: step["conclusion"]
            for step in job["steps"]
            if step["status"] == "completed"
        }
        suite_finished = (
            job["name"] in {"Rust", "Nightly Rust"}
            and completed_steps.get(SUITE_STEP) in {"success", "failure"}
        )
        skip_confirmed = (
            job["name"] == "Nightly preflight"
            and job["conclusion"] == "success"
            and completed_steps.get(SKIP_STEP) == "success"
        )
        if not (suite_finished or skip_confirmed):
            continue

        attempt_number = job["run_attempt"]
        if attempt_number > run["run_attempt"]:
            continue
        attempt = (
            run
            if attempt_number == run["run_attempt"]
            else api(f"{run_path}/attempts/{attempt_number}")
        )
        if (
            attempt["id"] == run["id"]
            and attempt["run_attempt"] == attempt_number
            and attempt["status"] == "completed"
            and attempt["conclusion"] in {"success", "failure"}
            and datetime.fromisoformat(attempt["updated_at"]) < before
        ):
            return True
    return False


def main():
    sha = os.environ["GITHUB_SHA"]
    run_id = int(os.environ["GITHUB_RUN_ID"])
    repo = os.environ["GITHUB_REPOSITORY"]
    if not re.fullmatch(r"[0-9a-f]{40}", sha) or run_id < 1:
        raise ValueError("Invalid scheduled run ID or commit SHA")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("Invalid repository")

    actions = f"repos/{repo}/actions"
    current = api(f"{actions}/runs/{run_id}")
    if (
        current["id"] != run_id
        or current["head_sha"] != sha
        or current["event"] != "schedule"
        or current["head_branch"] != BRANCH
        or current["path"] != ".github/workflows/ci.yml"
    ):
        raise ValueError("Run does not match this scheduled workflow and commit")

    workflow_id = current["workflow_id"]
    created_at = datetime.fromisoformat(current["created_at"])
    runs = api(
        f"{actions}/workflows/{workflow_id}/runs",
        branch=BRANCH,
        event="schedule",
        status="completed",
        created=f"<={current['created_at']}",
        per_page=100,
    )["workflow_runs"]
    if not isinstance(runs, list):
        raise ValueError("Invalid nightly workflow history response")

    run_suite = True
    for run in sorted(runs, key=lambda run: run["run_number"], reverse=True):
        if (
            run["id"] == run_id
            or run["run_number"] >= current["run_number"]
            or run["workflow_id"] != workflow_id
            or run["event"] != "schedule"
            or run["head_branch"] != BRANCH
            or run["status"] != "completed"
        ):
            continue
        if not re.fullmatch(r"[0-9a-f]{40}", run["head_sha"]):
            raise ValueError("Invalid commit SHA in nightly history")
        if completed_nightly(run, actions, created_at):
            run_suite = run["head_sha"] != sha
            print(f"Previous completed nightly: {run['id']} at {run['head_sha']}")
            break

    print("Run nightly suite" if run_suite else "Skip unchanged nightly suite")
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        print(f"run-suite={'true' if run_suite else 'false'}", file=output)


if __name__ == "__main__":
    main()

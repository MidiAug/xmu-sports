"""Development entry; uses task-local reader dependencies when available."""
from pathlib import Path
import sys

deps = Path(__file__).resolve().parents[3] / "artifacts" / "http-booking-deps"
if deps.is_dir():
    sys.path.insert(0, str(deps))

from xmu_http.app import main

if __name__ == "__main__":
    raise SystemExit(main())

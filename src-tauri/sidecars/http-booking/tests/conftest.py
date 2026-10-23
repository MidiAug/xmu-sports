import pytest


@pytest.fixture(autouse=True)
def no_real_reconciliation_delays(monkeypatch):
    # Exercise every polling attempt without sleeping in offline regression tests.
    monkeypatch.setattr("xmu_http.client.RECONCILE_DELAYS", (0, 0, 0, 0, 0))

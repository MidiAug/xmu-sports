"""Bounded retries for explicitly repeatable requests, never booking mutations."""
import random
import time
from datetime import datetime, timezone
from email.utils import parsedate_to_datetime

import requests

from .session import BookingError


RETRY_DELAYS = (1, 2)
SESSION_RETRIES = 6
RETRY_WINDOW_SECONDS = 60
REPEATABLE_PATHS = {
    "/index/Index/checkLogin", "/index/stadium/mySubscribe", "/index/stadium/subDetails",
    "/index/Stadium/getStadiumList", "/index/Stadium/getStadiumDetails",
    "/stadium/interval/getInterval", "/index/stadium/chooseVerify",
    "/index/Stadium/getVenueConfig",
}


def retry_after(value):
    if not value:
        return None
    try:
        return max(0, int(value))
    except (ValueError, TypeError):
        try:
            return max(0, (parsedate_to_datetime(value) - datetime.now(timezone.utc)).total_seconds())
        except (ValueError, TypeError, OverflowError):
            return None


def transient(error):
    if error.code in {"SERVER_BUSY", "RATE_LIMITED"}:
        return (error.details or {}).get("http_status") in {429, 500, 502, 503, 504}
    cause = error.__cause__
    return (error.code in {"HTTP_FAILED", "CAPTCHA_FETCH_FAILED"}
            and isinstance(cause, (requests.Timeout, requests.ConnectionError))
            and not isinstance(cause, (requests.exceptions.SSLError, requests.exceptions.ProxyError)))


class RequestRetry:
    def __init__(self, cancelled=None, progress=None):
        self.cancelled = cancelled
        self.progress = progress
        self.used = 0
        self.deadline = None

    def check_cancelled(self):
        if self.cancelled is not None and self.cancelled.is_set():
            raise BookingError("CANCELLED", "任务已停止，未继续请求")

    def call(self, operation, api, timeout=(8, 20), deadline=None):
        last_error = None
        for attempt in range(len(RETRY_DELAYS) + 1):
            self.check_cancelled()
            remaining = deadline - time.monotonic() if deadline is not None else None
            if remaining is not None and remaining <= 0:
                if last_error is not None:
                    raise last_error
                raise BookingError("ORDER_CHECK_TIMEOUT", "本轮查单时间已用完，不能据此认定预约不存在")
            current_timeout = tuple(min(value, remaining) for value in timeout) if remaining is not None else timeout
            try:
                return operation(current_timeout)
            except BookingError as error:
                last_error = error
                if not transient(error):
                    raise
                if self.deadline is None:
                    self.deadline = time.monotonic() + RETRY_WINDOW_SECONDS
                limit = min(self.deadline, deadline) if deadline is not None else self.deadline
                if attempt == len(RETRY_DELAYS) or self.used >= SESSION_RETRIES:
                    raise
                delay = max(RETRY_DELAYS[attempt] + random.uniform(0, 0.5),
                            (error.details or {}).get("retry_after_seconds") or 0)
                # Do not shorten the server's Retry-After to squeeze in a retry.
                if time.monotonic() + delay >= limit:
                    raise
                self.used += 1
                if self.progress:
                    self.progress({"request_retry": True, "retry_api": api,
                                   "retry_attempt": attempt + 1, "retry_total": len(RETRY_DELAYS),
                                   "retry_after_seconds": round(delay, 2), "retry_code": error.code})
                if self.cancelled is not None:
                    if self.cancelled.wait(delay):
                        raise BookingError("CANCELLED", "任务已停止，未重试请求") from error
                else:
                    time.sleep(delay)
                # Bound the next attempt too, including time already spent waiting.
                deadline = limit

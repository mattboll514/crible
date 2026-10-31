# SPDX-License-Identifier: Apache-2.0

"""Errors of the ``crible`` package.

:class:`BatchError` reports a batch that breaks a rule of
``docs/format-lot-v1.md``; it is raised before any file is written and has no
link to the HTTP API.

The other classes belong to the HTTP client and mirror the error table of
``docs/05-api.md`` (ADR-0112): one subclass per code, named ``<Code in
CamelCase>Error``. The subclass of ``timeout`` is :class:`RequestTimeoutError`,
so it does not mask the standard :class:`TimeoutError` that :meth:`wait` raises
at its own deadline. A code the client does not know raises the base
:class:`CribleError`, with ``code`` kept as the server sent it. A failure of the
transport raises :class:`CribleTransportError`. The client never retries: a
caller reads ``retry_after`` and decides.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any, Mapping

if TYPE_CHECKING:  # pragma: no cover - imported for type checking only.
    import httpx


class BatchError(Exception):
    """A batch breaks a rule of ``docs/format-lot-v1.md``.

    The writer raises this error before it writes the manifest, so a rejected
    batch is never presented as valid. The message names the offending value
    and where it comes from.
    """


class CribleError(Exception):
    """An error of the Crible API, or a failure that prevented the call.

    ``code`` is the code of ``docs/05-api.md`` (``None`` when the response has
    no conforming body), ``status`` the HTTP status (``None`` for a job error,
    which is not a request), ``details`` the object of the error body,
    ``request_id`` the identifier of ADR-0098 and ``retry_after`` the value of
    the ``Retry-After`` header, in seconds, when the response carries one.
    """

    def __init__(
        self,
        message: str = "",
        *,
        code: str | None = None,
        status: int | None = None,
        details: Mapping[str, Any] | None = None,
        request_id: str | None = None,
        retry_after: float | None = None,
    ) -> None:
        super().__init__(message)
        self.message = message
        self.code = code
        self.status = status
        self.details: dict[str, Any] = dict(details) if details else {}
        self.request_id = request_id
        self.retry_after = retry_after


class CribleTransportError(CribleError):
    """The request could not be completed.

    A connection failure or a timeout of ``httpx``, wrapped with the original
    exception in ``__cause__``. ``code`` is ``None``: no server error code was
    received.
    """


# One class per code of the error table of `docs/05-api.md`.
class InvalidRequestError(CribleError):
    """The body or a parameter is invalid (400)."""


class InvalidFilterError(CribleError):
    """A filter clause is outside the grammar of `docs/05-api.md` (400)."""


class UnauthorizedError(CribleError):
    """The API key is absent, malformed or unknown (401)."""


class ForbiddenError(CribleError):
    """The role of the API key is insufficient (403)."""


class UnknownIndexError(CribleError):
    """The index does not exist (404)."""


class NotFoundError(CribleError):
    """A document, a job, a batch directory or a route is not found (404)."""


class ImmutableSettingError(CribleError):
    """A frozen setting was modified (409)."""


class BatchConflictError(CribleError):
    """The same `batch_id` came with a different manifest (409)."""


class PayloadTooLargeError(CribleError):
    """The body exceeds the size limit (413)."""


class IncompatibleModelError(CribleError):
    """The model of a batch or of the targets of a search is incompatible (422)."""


class AcceptanceFailedError(CribleError):
    """The acceptance check of a batch was refused (422)."""


class OverloadedError(CribleError):
    """The write queue, the concurrency limit or the server is saturated (503)."""


class RequestTimeoutError(CribleError):
    """The search exceeded its `timeout_ms` (504)."""


class InternalError(CribleError):
    """An unexpected failure of the server (500)."""


#: Code of `docs/05-api.md` to its exception class (ADR-0112, decision 3).
CODE_ERRORS: dict[str, type[CribleError]] = {
    "invalid_request": InvalidRequestError,
    "invalid_filter": InvalidFilterError,
    "unauthorized": UnauthorizedError,
    "forbidden": ForbiddenError,
    "unknown_index": UnknownIndexError,
    "not_found": NotFoundError,
    "immutable_setting": ImmutableSettingError,
    "batch_conflict": BatchConflictError,
    "payload_too_large": PayloadTooLargeError,
    "incompatible_model": IncompatibleModelError,
    "acceptance_failed": AcceptanceFailedError,
    "overloaded": OverloadedError,
    "timeout": RequestTimeoutError,
    "internal": InternalError,
}


def error_for_response(response: httpx.Response) -> CribleError:
    """Builds the exception of an HTTP error response.

    The body follows ADR-0101: ``{"error": {"code", "message", "details",
    "request_id"}}``. A response without that body raises the base
    :class:`CribleError` with ``code`` at ``None`` and the status, so a proxy or
    a static layer that answers before the API still gives a type and a status
    to the caller.
    """
    status = response.status_code
    retry_after = _retry_after(response)
    error = _error_member(response)
    if error is None:
        return CribleError(
            f"HTTP {status} without an error body of the API",
            status=status,
            retry_after=retry_after,
        )
    return _typed(
        error.get("code"),
        message=_text(error.get("message")),
        status=status,
        details=error.get("details"),
        request_id=_text(error.get("request_id")),
        retry_after=retry_after,
    )


def error_for_job(error: Mapping[str, Any]) -> CribleError:
    """Builds the exception of the ``error`` member of a failed job.

    A job is not a request: the error carries no ``request_id`` and no HTTP
    status (ADR-0103, ADR-0112).
    """
    return _typed(
        error.get("code"),
        message=_text(error.get("message")),
        status=None,
        details=error.get("details"),
        request_id=None,
        retry_after=None,
    )


def _typed(
    code: Any,
    *,
    message: str,
    status: int | None,
    details: Any,
    request_id: str | None,
    retry_after: float | None,
) -> CribleError:
    """Instantiates the class of ``code``, or the base class when it is unknown."""
    error_class = CODE_ERRORS.get(code, CribleError) if isinstance(code, str) else CribleError
    return error_class(
        message,
        code=code if isinstance(code, str) else None,
        status=status,
        details=details if isinstance(details, Mapping) else None,
        request_id=request_id,
        retry_after=retry_after,
    )


def _error_member(response: httpx.Response) -> Mapping[str, Any] | None:
    """Returns the ``error`` object of the body, or ``None`` when it is absent."""
    try:
        body = response.json()
    except ValueError:
        return None
    if not isinstance(body, Mapping):
        return None
    error = body.get("error")
    if not isinstance(error, Mapping) or not isinstance(error.get("code"), str):
        return None
    return error


def _retry_after(response: httpx.Response) -> float | None:
    """Returns the ``Retry-After`` header in seconds, or ``None``."""
    value = response.headers.get("retry-after")
    if value is None:
        return None
    try:
        return float(value)
    except ValueError:
        return None


def _text(value: Any) -> str:
    """Returns ``value`` when it is a string, and the empty string otherwise."""
    return value if isinstance(value, str) else ""

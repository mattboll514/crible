"""Fictional ``PROD_PGOPTIONS`` values shared by the MES-00 tests.

The real value lives in ``.env.prod`` and must never appear in a versioned
file (ADR-0032, rule 11). The tests only need the *shape* of the value, so they
use fictional ones: the read-only setting is spelled with ``--`` instead of the
production form, which exercises the same parser without reproducing the
production string.

The three spelling cases are kept:

* ``VALID``: accepted, with an ordinary option first;
* ``SEQUENTIAL_OFF`` and ``SPACED_OFF``: a later ``=off`` cancels the earlier
  ``=on`` (separate spellings: ``-c`` and ``--`` with spaces);
* ``ATTACHED_OFF``: attached spelling, ``-cdefault_transaction_read_only=off``.

No value here is read from the environment: the tests run in CI without
``.env.prod``.
"""

# Accepted: one ordinary option, then the read-only setting.
VALID = "-c statement_timeout=2000 --default_transaction_read_only=on"

# Accepted too, and it carries a free-form setting the guard must not echo.
VALID_WITH_APP_NAME = VALID + " -c application_name=mes00-test"

# Refused: a later =off cancels the read-only setting, with two -c options.
SEQUENTIAL_OFF = (
    "--default_transaction_read_only=on -c default_transaction_read_only=off"
)

# Refused: same, written with the long spelling and spaces around =.
SPACED_OFF = (
    "--default_transaction_read_only = on "
    "--default_transaction_read_only = off"
)

# Refused: attached spelling, no space after -c.
ATTACHED_OFF = "-cdefault_transaction_read_only=off"

# Refused: no read-only setting at all.
NOT_READ_ONLY = "-c statement_timeout=1000"

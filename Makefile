# SPDX-License-Identifier: Apache-2.0

CARGO ?= cargo

.PHONY: check fmt lint test

check: fmt lint test

fmt:
	$(CARGO) fmt --all -- --check

lint:
	$(CARGO) clippy --all-targets -- -D warnings

test:
	$(CARGO) test

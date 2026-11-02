// SPDX-License-Identifier: Apache-2.0

//! The example configuration of the Docker image is accepted by the loader.
//!
//! References: the task `DEFINIR-VOLUMES-ET-CONFIG`
//! (`docs/phases/P8-docker-et-deploiement.md`) and `docs/07-stack-et-deploiement.md`
//! ("Configuration de l'instance", "Docker"). The file `docker/crible.example.toml`
//! is mounted as `/etc/crible/crible.toml` by `docker/README.md`; this test reads
//! it with the loader of the server, so a key added to or removed from the
//! closed list of ADR-0097 cannot leave the example silently stale.

// A test may panic: the failure of the fixture is the failure of the test, and
// the helper below is not a `#[test]` function, so the workspace lint does not
// cover it.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::Path;

use crible_server::Config;

/// Path of the example, from the manifest directory of this crate.
fn example_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docker/crible.example.toml")
        .canonicalize()
        .expect("docker/crible.example.toml must exist at the root of the repository")
}

#[test]
fn the_docker_example_configuration_is_accepted() {
    // `load_with_env` is the loader of `crible serve`, with the environment
    // passed instead of read: the test never depends on the variables of the
    // machine that runs it (ADR-0097).
    let config =
        Config::load_with_env(&example_path(), std::iter::empty()).unwrap_or_else(|error| {
            panic!(
                "docker/crible.example.toml must be accepted by the loader, got {}: {}",
                error.code().as_str(),
                error.message()
            )
        });

    // The three roots are the directories the image declares (docs/07, "Docker").
    assert_eq!(config.paths.data, Path::new("/data"));
    assert_eq!(config.paths.models, Path::new("/models"));
    assert_eq!(config.paths.exchange, Path::new("/exchange"));

    // The example holds no key of a deployment: it is the closed-by-default
    // instance, which starts and answers `401` on the authenticated routes.
    assert!(
        config.auth.keys.is_empty(),
        "the example must not declare an API key"
    );

    // The stop timeout of the container is above this bound (docs/07:129,
    // ADR-0104); `docker/README.md` passes `--stop-timeout 65`.
    assert_eq!(config.limits.wait_timeout_ms, 60_000);
}

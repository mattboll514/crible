// SPDX-License-Identifier: Apache-2.0

//! The example configuration of the Docker image is accepted by the loader.
//!
//! References: the task `DEFINIR-VOLUMES-ET-CONFIG`
//! (`docs/phases/P8-docker-et-deploiement.md`) and `docs/07-stack-et-deploiement.md`
//! ("Configuration de l'instance", "Docker"). The file `docker/crible.example.toml`
//! is mounted as `/etc/crible/crible.toml` by `docker/README.md`; this test reads
//! it with the loader of the server, so a key added to or removed from the
//! closed list of ADR-0097 cannot leave the example silently stale.
//!
//! The task `COMPOSE-EXEMPLE` adds `docker/crible.demo.toml`, the configuration
//! of `docker/compose.example.yml`; it gets the same treatment, because nothing
//! else in the repository loads it.

// A test may panic: the failure of the fixture is the failure of the test, and
// the helper below is not a `#[test]` function, so the workspace lint does not
// cover it.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::Path;

use crible_server::Config;
use crible_server::auth::hex_encode;
use crible_server::config::Role;
use sha2::{Digest, Sha256};

/// Path of a file of `docker/` that the tests load, from the manifest directory.
fn docker_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docker")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|error| {
            panic!("docker/{name} must exist at the root of the repository: {error}")
        })
}

/// The demonstration key the `client` service sends, read from the Compose file.
///
/// The Compose file is the only source: repeating the key here would let the two
/// drift apart without a red test.
fn compose_demo_key() -> String {
    let path = docker_path("compose.example.yml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
    text.lines()
        .find_map(|line| {
            let value = line.trim().strip_prefix("CRIBLE_API_KEY:")?;
            Some(value.trim().trim_matches('"').trim_matches('\'').to_owned())
        })
        .unwrap_or_else(|| {
            panic!("docker/compose.example.yml must set the demonstration key in `CRIBLE_API_KEY`")
        })
}

#[test]
fn the_docker_example_configuration_is_accepted() {
    // `load_with_env` is the loader of `crible serve`, with the environment
    // passed instead of read: the test never depends on the variables of the
    // machine that runs it (ADR-0097).
    let config = Config::load_with_env(&docker_path("crible.example.toml"), std::iter::empty())
        .unwrap_or_else(|error| {
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

#[test]
fn the_compose_demo_configuration_is_accepted() {
    // `docker/crible.demo.toml` is mounted by `docker/compose.example.yml`, and
    // only the run of the demonstration would notice a key the loader rejects.
    let config = Config::load_with_env(&docker_path("crible.demo.toml"), std::iter::empty())
        .unwrap_or_else(|error| {
            panic!(
                "docker/crible.demo.toml must be accepted by the loader, got {}: {}",
                error.code().as_str(),
                error.message()
            )
        });

    // Same three roots as the example: the demo replaces nothing but the keys.
    assert_eq!(config.paths.data, Path::new("/data"));
    assert_eq!(config.paths.models, Path::new("/models"));
    assert_eq!(config.paths.exchange, Path::new("/exchange"));

    // One read key, the one the `client` service sends. Its digest is compared
    // to the demonstration key read in `docker/compose.example.yml`, so the two
    // files cannot drift apart and turn the demo into a `401`.
    assert_eq!(
        config.auth.keys.len(),
        1,
        "the demo declares exactly one key"
    );
    let key = &config.auth.keys[0];
    assert_eq!(key.name, "compose-demo");
    assert_eq!(key.role, Role::Read);
    let demo_key = compose_demo_key();
    assert_eq!(
        key.sha256,
        hex_encode(&Sha256::digest(demo_key.as_bytes())),
        "the digest of docker/crible.demo.toml must be the one of the key of \
         docker/compose.example.yml"
    );

    // Same stop timeout as the example, for the same reason; the compose
    // service passes `stop_grace_period: 65s`.
    assert_eq!(config.limits.wait_timeout_ms, 60_000);
}

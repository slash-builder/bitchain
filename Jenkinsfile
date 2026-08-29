// Centralized, pinned Rust toolchain image — bump here to move every stage at once.
// Build, test, and publish run inside this container so the Rust version stays
// decoupled from the Jenkins agent host. Pinned to latest stable as of 2026-06.
def RUST_IMAGE = 'rust:1.94'

pipeline {
  // Pin to the Docker-capable Linux build host. `agent any` can land on
  // a node with no `docker` CLI at all (confirmed live 2026-08-26 across
  // several kit repos: the first real CI run failed with "docker: not
  // found" before any `docker { image ... }` stage agent ever got a
  // chance to run) -- same fix already applied in quickring/gateway,
  // quickring/hub, and slash-builder/substrate-kit's own Jenkinsfiles
  // for the identical reason.
  agent { label 'linux-build' }

  environment {
    NEXUS_URL  = 'https://nexus.softsurve.com'
    CARGO_HOME = "${WORKSPACE}/.cargo"
  }

  stages {

    // ── Pre-flight ─────────────────────────────────────────────────────────────
    stage('Pre-flight') {
      steps {
        script {
          def msg = sh(script: 'git log -1 --pretty=%B', returnStdout: true).trim()
          if (msg.contains('[skip ci]')) {
            currentBuild.result = 'NOT_BUILT'
            error('Commit contains [skip ci] — aborting.')
          }
        }
      }
    }

    // ── Build & Test ───────────────────────────────────────────────────────────
    stage('Build & Test') {
      agent {
        docker {
          image RUST_IMAGE
          reuseNode true
        }
      }
      steps {
        sh '''
          rustup component add rustfmt clippy
          cargo fmt --check
          cargo clippy -- -D warnings
          cargo test
          cargo build --release
        '''
      }
      post {
        success {
          archiveArtifacts artifacts: 'target/release/bitchain', allowEmptyArchive: false
        }
      }
    }

    // ── Publish → Nexus (Cargo) ────────────────────────────────────────────────
    stage('Publish') {
      when { branch 'main' }
      agent {
        docker {
          image RUST_IMAGE
          reuseNode true
        }
      }
      steps {
        withCredentials([
          usernamePassword(
            credentialsId: 'nexus-credentials',
            usernameVariable: 'NEXUS_USER',
            passwordVariable: 'NEXUS_PASS'
          ),
          usernamePassword(
            credentialsId: 'github-token',
            usernameVariable: 'GIT_USER',
            passwordVariable: 'GIT_TOKEN'
          )
        ]) {
          sh '''
            git config user.email "ci@softsurve.com"
            git config user.name  "Sol CI"

            set -eu

            origin=$(git config --get remote.origin.url)
            path=$(printf '%s' "$origin" | sed -E 's#(git@github.com:|https://github.com/)##; s#[.]git$##')
            remote="https://${GIT_USER}:${GIT_TOKEN}@github.com/${path}.git"

            # Tags are the version source of truth — bump the patch above the latest vX.Y.Z.
            git fetch --tags --quiet "$remote" || true
            latest=$(git tag -l 'v*.*.*' | sort -V | tail -1)
            if [ -z "$latest" ]; then
              next="0.1.0"
            else
              v=${latest#v}; maj=${v%%.*}; rest=${v#*.}; min=${rest%%.*}; pat=${rest##*.}
              next="${maj}.${min}.$((pat + 1))"
            fi
            echo "Publishing ${path} v${next}"

            # Set the crate version for this publish (ephemeral; tags stay the source of truth).
            if [ -f Cargo.toml ]; then
              sed -i 's/^version = ".*"/version = "'"$next"'"/' Cargo.toml
            fi

            # Cargo registry auth.
            #
            # Only append [registries.lockamy] here if this repo does not
            # ALSO git-track it in .cargo/config.toml (the pattern a repo
            # needs the moment it *consumes* another lockamy-registry crate,
            # not just publishes one -- see CLUS-62). If it ever gains that
            # tracked file, re-appending the same key here duplicates it and
            # cargo publish fails with a TOML parse error before reaching
            # Nexus. Same bug found and fixed identically in identity-kit,
            # storage-kit, and fabric-kit, 2026-08-29 -- fixed here
            # proactively before this repo hits it too.
            mkdir -p "$CARGO_HOME"
            if [ -f .cargo/config.toml ] && grep -q '^\[registries.lockamy\]' .cargo/config.toml; then
              cat >> "$CARGO_HOME/config.toml" <<EOF

[registries.lockamy-hosted]
index = "sparse+${NEXUS_URL}/repository/cargo-hosted/"

[registry]
default = "lockamy"
EOF
            else
              cat >> "$CARGO_HOME/config.toml" <<EOF
[registries.lockamy]
index = "sparse+${NEXUS_URL}/repository/cargo-group/"

[registries.lockamy-hosted]
index = "sparse+${NEXUS_URL}/repository/cargo-hosted/"

[registry]
default = "lockamy"
EOF
            fi
            # Nexus speaks HTTP Basic; cargo sends the token verbatim as the Authorization header,
            # so the token must be "Basic <base64(user:pass)>" — not a bare user:pass.
            BASIC="Basic $(printf '%s:%s' "${NEXUS_USER}" "${NEXUS_PASS}" | base64 -w0)"
            printf '[registries.lockamy]\ntoken = "%s"\n[registries.lockamy-hosted]\ntoken = "%s"\n' \
              "${BASIC}" "${BASIC}" >> "$CARGO_HOME/credentials.toml"
            chmod 0600 "$CARGO_HOME/credentials.toml"

            # Publish (allow-dirty: the version was just set in-tree).
            # Idempotent: a version already present on Nexus is treated as success,
            # so re-running a build that already published doesn't fail the pipeline.
            set +e
            pub_out=$(cargo publish --registry lockamy-hosted --allow-dirty 2>&1)   # resolve via group (default), publish to hosted
            pub_rc=$?
            set -e
            printf '%s\n' "$pub_out"
            if [ "$pub_rc" -ne 0 ]; then
              if printf '%s' "$pub_out" | grep -qiE 'already (exists|uploaded)|already been uploaded|version .* is already'; then
                echo "v${next} already published — treating as success (idempotent)."
              else
                exit "$pub_rc"
              fi
            fi

            # Record the release as a tag and push it back to origin (skip if it already exists).
            if git rev-parse -q --verify "refs/tags/v${next}" >/dev/null; then
              echo "Tag v${next} already exists locally — skipping tag."
            else
              git tag "v${next}"
              git push "$remote" "v${next}" || echo "Tag push skipped (already on origin)."
            fi
          '''
        }
      }
    }

  }

  post {
    success {
      echo 'bitchain published to nexus.softsurve.com/repository/cargo-hosted/'
    }
    failure {
      echo 'Pipeline failed.'
    }
    always {
      cleanWs()
    }
  }
}

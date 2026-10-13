# The Linux environment of CI's ubuntu-latest jobs, for `pnpm run linux` (tooling/linux.mjs).
# Versions follow the repository pins: .nvmrc, package.json packageManager, rust-toolchain.toml.
FROM ubuntu:24.04

ARG NODE_VERSION
ARG PNPM_VERSION
ARG RUST_VERSION
ENV DEBIAN_FRONTEND=noninteractive

# Build tools, Xvfb for Electron, and the shared libraries Electron and Chromium load.
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git rsync xz-utils build-essential pkg-config python3 \
      xvfb xauth dbus-x11 \
      libgtk-3-0t64 libnss3 libxss1 libasound2t64 libgbm1 libxshmfence1 libdrm2 \
      libxkbcommon0 libatk-bridge2.0-0t64 libcups2t64 libsecret-1-0 libnotify4 \
    && rm -rf /var/lib/apt/lists/*

RUN arch="$(dpkg --print-architecture | sed 's/amd64/x64/')" \
    && curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-${arch}.tar.xz" \
       | tar -xJ -C /usr/local --strip-components=1 \
    && npm install -g "pnpm@${PNPM_VERSION}" \
    && node --version && pnpm --version

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "${RUST_VERSION}" \
      --component rustfmt,clippy,llvm-tools-preview --target wasm32-unknown-unknown \
    && curl -fsSL https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash \
    && cargo binstall -y --locked cargo-llvm-cov cargo-deny \
    && mkdir -p /usr/local/cargo/registry \
    && chmod -R a+rwX /usr/local/cargo /usr/local/rustup

# Electron refuses to run as root; work as the image's unprivileged `ubuntu` user (uid 1000).
# The cache volumes mount over these directories and inherit their ownership.
RUN mkdir -p /work /pnpm-store /results && chown ubuntu:ubuntu /work /pnpm-store /results
USER ubuntu
WORKDIR /work

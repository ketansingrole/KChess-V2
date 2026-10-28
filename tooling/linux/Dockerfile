# The Linux environment of CI's ubuntu-latest jobs, to run `cargo xtask check` (or any command)
# on another machine:
#   docker build --build-arg RUST_VERSION="$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)" -t kchess-linux tooling/linux
#   docker run --rm -v "$PWD":/work kchess-linux cargo xtask check
FROM ubuntu:24.04

ARG RUST_VERSION
ENV DEBIAN_FRONTEND=noninteractive

# Build tools and the headers the app (apps/gpui) builds against: X11/XCB, Wayland, xkbcommon,
# fontconfig, Vulkan and ALSA (sound).
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git build-essential pkg-config python3 \
      libasound2-dev libfontconfig-dev libwayland-dev libx11-xcb-dev libxcb1-dev \
      libxkbcommon-dev libxkbcommon-x11-dev libvulkan-dev \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "${RUST_VERSION}" \
      --component rustfmt,clippy,llvm-tools-preview \
    && curl -fsSL https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash \
    && cargo binstall -y --locked cargo-llvm-cov cargo-deny \
    && chmod -R a+rwX /usr/local/cargo /usr/local/rustup

USER ubuntu
WORKDIR /work

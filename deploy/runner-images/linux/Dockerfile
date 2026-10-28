# The job image of the Linux build runners (label linux-x64): everything the
# .gitea/workflows jobs need, preinstalled so a job starts building at once.
#
# Ubuntu 22.04 keeps the Linux desktop packages on the glibc they shipped
# with before (the GitHub builds used the same release). Every download is
# pinned and checked against its SHA-256 checksum.
#
# Built and pushed by .gitea/workflows/runner-image.yml, or once by hand:
#   docker build -t 192.168.1.58:3002/mavingaw/mujin-runner-linux:1 deploy/runner-images/linux
#   docker push 192.168.1.58:3002/mavingaw/mujin-runner-linux:1
FROM ubuntu:22.04@sha256:e0f80c829509ef4df9e76bd8944eb7b8ec25cbb379b112a7f9448b2c77a77c9b

ARG NODE_VERSION=24.22.1
ARG NODE_SHA256=6c172a9ab6718f7b801fb5a7bd486af2ee5e32758086a04b84fcce2aa77095e6
ARG BUN_VERSION=1.4.2
ARG BUN_SHA256=36368faef7527875d5ffa52e53cd48021741f2a83eb6208a8dd64068d422a913
ARG RUSTUP_VERSION=1.29.2
ARG RUSTUP_SHA256=5be191e8c70a5007074254cb74d78dbfdd25cd5a61c27de7b8e5410122ec24f4
ARG RUST_VERSION=1.99.0
ARG CARGO_AUDIT_VERSION=0.22.2
ARG ANDROID_CMDLINE_TOOLS=16111833
ARG ANDROID_CMDLINE_TOOLS_SHA256=0877a1d048fe4a24efe2eff536ca4223f7adeb58648bb81909d33c446918cfa8
ARG DOCKER_VERSION=29.9.2
ARG DOCKER_SHA256=9ac225999fd5e863f7694c728ef23d480616b1e5639328fa309befd4243d69eb
ARG BUILDX_VERSION=0.38.1
ARG BUILDX_SHA256=e25d5cd675092bb5d1d03632fdc46cdf3fede7b514dc5f023a105e00d3ce5ad4
ARG CRANE_VERSION=0.22.1
ARG CRANE_SHA256=0ab7a1d6932a213aed964ce97666c3077fe691c8606413674a8b3e0b9ec4cda0
ARG TRIVY_VERSION=0.76.1
ARG TRIVY_SHA256=c7cbe3137fb8a48a17574bede3dd6beb16742d1e9772929940fc90082c39d9c0

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    ANDROID_HOME=/opt/android-sdk \
    ANDROID_SDK_ROOT=/opt/android-sdk \
    JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64 \
    PIPX_BIN_DIR=/usr/local/bin \
    EXPO_NO_TELEMETRY=1 \
    NEXT_TELEMETRY_DISABLED=1
ENV PATH=/usr/local/cargo/bin:/opt/android-sdk/cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:$PATH

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

# Build tools for the backend (nasm for the audio codecs), the Linux desktop
# packages (patchelf, dpkg-dev, file), the arm64 cross build, the Home
# Assistant and enhancer tests (python3, pipx, ffmpeg) and the release jobs
# (jq, minisign, zstd).
RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    build-essential ca-certificates cmake curl dpkg-dev ffmpeg file \
    g++-aarch64-linux-gnu gcc-aarch64-linux-gnu git git-lfs jq libc6-dev-arm64-cross \
    libcrypt1 libssl-dev libxml2-utils minisign nasm ninja-build openjdk-21-jdk-headless \
    openssh-client patchelf pipx pkg-config python3 python3-venv rsync sudo unzip \
    xz-utils zip zstd \
  && rm -rf /var/lib/apt/lists/*

RUN curl -fsSLo /tmp/node.tar.xz "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
  && echo "${NODE_SHA256}  /tmp/node.tar.xz" | sha256sum --check --strict \
  && tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 --exclude='*.md' --exclude=LICENSE \
  && rm /tmp/node.tar.xz \
  && node --version

RUN curl -fsSLo /tmp/bun.zip "https://github.com/oven-sh/bun/releases/download/bun-v${BUN_VERSION}/bun-linux-x64.zip" \
  && echo "${BUN_SHA256}  /tmp/bun.zip" | sha256sum --check --strict \
  && unzip -j /tmp/bun.zip bun-linux-x64/bun -d /usr/local/bin \
  && ln -s bun /usr/local/bin/bunx \
  && rm /tmp/bun.zip \
  && bun --version

RUN curl -fsSLo /tmp/rustup-init "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
  && echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum --check --strict \
  && chmod +x /tmp/rustup-init \
  && /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain "${RUST_VERSION}" \
    --component rustfmt --component clippy --target aarch64-unknown-linux-gnu \
  && rm /tmp/rustup-init \
  && cargo install cargo-audit --locked --version "${CARGO_AUDIT_VERSION}" \
  && rm -rf "${CARGO_HOME}/registry" "${CARGO_HOME}/git" \
  && chmod -R a+rwX "${RUSTUP_HOME}" "${CARGO_HOME}"

# The SDK packages the generated Android project asks for (React Native's
# gradle/libs.versions.toml: compile SDK 36, build tools 36.0.0, NDK
# 27.1.12297006, CMake 3.30.5). Gradle downloads anything else on demand.
RUN curl -fsSLo /tmp/cmdline-tools.zip "https://dl.google.com/android/repository/commandlinetools-linux-${ANDROID_CMDLINE_TOOLS}_latest.zip" \
  && echo "${ANDROID_CMDLINE_TOOLS_SHA256}  /tmp/cmdline-tools.zip" | sha256sum --check --strict \
  && mkdir -p "${ANDROID_HOME}/cmdline-tools" \
  && unzip -q /tmp/cmdline-tools.zip -d "${ANDROID_HOME}/cmdline-tools" \
  && mv "${ANDROID_HOME}/cmdline-tools/cmdline-tools" "${ANDROID_HOME}/cmdline-tools/latest" \
  && rm /tmp/cmdline-tools.zip \
  && (yes || true) | sdkmanager --licenses >/dev/null \
  && sdkmanager --install \
    "platform-tools" "platforms;android-36" "build-tools;36.0.0" \
    "ndk;27.1.12297006" "cmake;3.30.5" >/dev/null \
  && chmod -R a+rwX "${ANDROID_HOME}"

# Docker CLI and buildx for the container image job (the runner mounts the
# host's Docker socket), crane to copy and tag images between registries,
# Trivy to scan them.
RUN curl -fsSLo /tmp/docker.tgz "https://download.docker.com/linux/static/stable/x86_64/docker-${DOCKER_VERSION}.tgz" \
  && echo "${DOCKER_SHA256}  /tmp/docker.tgz" | sha256sum --check --strict \
  && tar -xzf /tmp/docker.tgz -C /usr/local/bin --strip-components=1 docker/docker \
  && mkdir -p /usr/local/lib/docker/cli-plugins \
  && curl -fsSLo /usr/local/lib/docker/cli-plugins/docker-buildx "https://github.com/docker/buildx/releases/download/v${BUILDX_VERSION}/buildx-v${BUILDX_VERSION}.linux-amd64" \
  && echo "${BUILDX_SHA256}  /usr/local/lib/docker/cli-plugins/docker-buildx" | sha256sum --check --strict \
  && chmod +x /usr/local/lib/docker/cli-plugins/docker-buildx \
  && curl -fsSLo /tmp/crane.tgz "https://github.com/google/go-containerregistry/releases/download/v${CRANE_VERSION}/go-containerregistry_Linux_x86_64.tar.gz" \
  && echo "${CRANE_SHA256}  /tmp/crane.tgz" | sha256sum --check --strict \
  && tar -xzf /tmp/crane.tgz -C /usr/local/bin crane \
  && curl -fsSLo /tmp/trivy.tgz "https://github.com/aquasecurity/trivy/releases/download/v${TRIVY_VERSION}/trivy_${TRIVY_VERSION}_Linux-64bit.tar.gz" \
  && echo "${TRIVY_SHA256}  /tmp/trivy.tgz" | sha256sum --check --strict \
  && tar -xzf /tmp/trivy.tgz -C /usr/local/bin trivy \
  && rm /tmp/docker.tgz /tmp/crane.tgz /tmp/trivy.tgz \
  && docker --version && docker buildx version && crane version && trivy --version

# Cross linker for the optional arm64 Linux desktop build.
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++ \
    AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar

LABEL org.opencontainers.image.title="Mujin Music Linux build runner" \
  org.opencontainers.image.licenses="GPL-3.0-only"

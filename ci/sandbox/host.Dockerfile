# The sandbox's mesimon host: Debian 12 with the distro's tmux, and the Rust
# toolchain that builds mesimon from the mounted checkout. Nothing of mesimon
# is baked in: `ci/sandbox.sh build` compiles into the `target` volume and the
# container runs that binary, so a rebuild needs no new image.
ARG RUST_VERSION=1
FROM rust:${RUST_VERSION}-bookworm
RUN apt-get update \
    && apt-get install -y --no-install-recommends tmux git python3 procps less \
    && rm -rf /var/lib/apt/lists/*
# Claude Code, outside HOME (a volume over /root would hide it). The image
# keeps the version it was built with; a newer one is
# `ci/sandbox.sh compose build --no-cache host`, then `ci/sandbox.sh up`.
RUN HOME=/opt/claude bash -c 'curl -fsSL https://claude.ai/install.sh | bash' \
    && ln -s /opt/claude/.local/bin/claude /usr/local/bin/claude \
    && claude --version
# A ticket's worktree is a commit away from merging; a container has no identity.
RUN git config --global user.name "sandbox" \
    && git config --global user.email "sandbox@mesimon.invalid" \
    && git config --global init.defaultBranch main \
    && git config --global --add safe.directory '*'
ENV LANG=C.UTF-8 \
    SHELL=/bin/bash \
    CARGO_TARGET_DIR=/target \
    PATH=/target/debug:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
WORKDIR /root
CMD ["sleep", "infinity"]

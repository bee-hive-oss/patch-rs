FROM docker.io/library/rust:1.100.0-slim-trixie

# GNU tools the GNU patch 2.8 suite requires (diff, ed, sed, stat), plus what
# is needed to build the GNU patch 2.8 reference from its checksummed tarball.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ed diffutils sed coreutils python3 ca-certificates curl make xz-utils libattr1-dev \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy

ARG PATCH_VERSION=2.8
ARG PATCH_SHA256=308a4983ff324521b9b21310bfc2398ca861798f02307c79eb99bb0e0d2bf980
RUN curl -fsSL -o /tmp/patch.tar.gz "https://ftp.gnu.org/gnu/patch/patch-${PATCH_VERSION}.tar.gz" \
    && echo "${PATCH_SHA256}  /tmp/patch.tar.gz" | sha256sum -c \
    && mkdir /tmp/pb && tar -xzf /tmp/patch.tar.gz -C /tmp/pb \
    && cd /tmp/pb/patch-${PATCH_VERSION} && ./configure --prefix=/usr/local >/dev/null && make -j"$(nproc)" >/dev/null && make install >/dev/null \
    && /usr/local/bin/patch --version | head -n 1 \
    && cd / && rm -rf /tmp/pb /tmp/patch.tar.gz

WORKDIR /work

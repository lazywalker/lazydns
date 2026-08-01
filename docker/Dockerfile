# lazydns Dockerfile
# Multi-stage build: download binary in first stage, use scratch in second stage
# How to run:
#
# Need to map config volume and ports
# 
# docker run -d \
#   --name lazydns \
#   -p 53:53/udp -p 53:53/tcp \
#   -p 853:853/tcp -p 443:443/tcp \
#   -p 784:784/tcp -p 8000:8000/tcp -p 8001:8001/tcp \
#   -e TZ=Asia/Shanghai \
#   -v /path/to/config:/etc/lazydns \
#   lazywalker/lazydns:latest

# Stage 1: Downloader and Preparer
FROM alpine:3.23 AS downloader

# Install curl and other tools for downloading and preparing
RUN apk add --no-cache curl tzdata

# Download latest release binary based on architecture
ARG LAZYDNS_VERSION=latest
ARG TARGETARCH
RUN set -ex; \
    case "$TARGETARCH" in \
    amd64) ARCH="x86_64" ;; \
    arm64) ARCH="aarch64" ;; \
    arm) ARCH="arm" ;; \
    *) echo "Unsupported architecture: $TARGETARCH" && exit 1 ;; \
    esac; \
    if [ "$TARGETARCH" = "arm" ]; then \
      SUFFIX="arm-unknown-linux-musleabihf"; \
    else \
      SUFFIX="aarch64-unknown-linux-musl"; \
    fi; \
    if [ "${LAZYDNS_VERSION}" = "latest" ]; then \
      VER=$(curl -s https://api.github.com/repos/lazywalker/lazydns/releases/latest | grep -m1 '"tag_name"' | sed -E 's/.*"v?([^"]+)".*/\1/'); \
    else \
      VER="${LAZYDNS_VERSION#v}"; \
    fi; \
    BINARY_NAME="lazydns-full-${VER}-${SUFFIX}.tar.gz"; \
    BASE_URL="https://github.com/lazywalker/lazydns/releases"; \
    URL="${BASE_URL}/download/v${VER}/${BINARY_NAME}"; \
    echo "Downloading from ${URL}"; \
    curl -L -o lazydns-full.tar.gz "${URL}" \
    && tar -xzf lazydns-full.tar.gz \
    && chmod +x lazydns


# Set timezone (can be overridden at runtime)
ARG TZ=Asia/Shanghai
RUN echo "$TZ" > /etc/timezone

# Prepare application files
RUN mkdir -p /etc/lazydns
ADD config.yaml /etc/lazydns/config.yaml
RUN sed -i 's/5354/53/g' /etc/lazydns/config.yaml || true
RUN sed -i 's/8443/443/g' /etc/lazydns/config.yaml || true
RUN sed -i 's/8853/853/g' /etc/lazydns/config.yaml || true
RUN sed -i 's/8784/784/g' /etc/lazydns/config.yaml || true
# Stage 2: Runtime (scratch - minimal)
FROM scratch

# Copy timezone and zoneinfo from the downloader stage
COPY --from=downloader /usr/share/zoneinfo /usr/share/zoneinfo
COPY --from=downloader /etc/timezone /etc/timezone
COPY --from=downloader /etc/lazydns /etc/lazydns

# Copy the binary
COPY --from=downloader /lazydns /lazydns

# Set environment variables
ENV TZ=Asia/Shanghai

# Expose DNS ports
EXPOSE 53/udp 53/tcp

# Expose DoT port
EXPOSE 853/tcp

# Expose DoH port
EXPOSE 443/tcp

# Expose DoQ port
EXPOSE 784/tcp

# Expose HTTP management ports
EXPOSE 8000/tcp
# Expose Prometheus metrics port
EXPOSE 8001/tcp

WORKDIR /etc/lazydns

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/lazydns", "--version"] || exit 1

# Default command
CMD ["/lazydns", "start", "-d", "/etc/lazydns"]


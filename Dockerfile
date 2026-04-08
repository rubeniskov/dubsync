# Use pixi base image for building
FROM ghcr.io/prefix-dev/pixi:latest AS build

# Set working directory
WORKDIR /app

# Copy the entire workspace including the local dependency
COPY . .

# Install dependencies and build in release mode
# Pixi will install rust, ffmpeg, cuda libs, etc. defined in pixi.toml
RUN pixi run build

# Use a clean CUDA-ready runtime image
FROM nvidia/cuda:12.4.1-base-ubuntu22.04

# Install minimal runtime dependencies (ffmpeg is managed by pixi, but we need libs)
RUN apt-get update && apt-get install -y --no-install-recommends \
    libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the built binary and the pixi environment from the build stage
COPY --from=build /app/target/release/dubsync /app/dubsync
COPY --from=build /app/.pixi/envs/default /app/env

# Set up environment variables for the runtime
ENV LD_LIBRARY_PATH="/app/env/lib:${LD_LIBRARY_PATH}"
ENV PATH="/app/env/bin:${PATH}"

# Entrypoint
ENTRYPOINT ["/app/dubsync"]

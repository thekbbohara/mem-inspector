# A plain Ubuntu 24.04 desktop baseline (GTK3, which every desktop has) with
# no WebKitGTK: proves the AppImage carries its own WebKit and runs on a
# machine that never saw the source.
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      xvfb xdotool x11-apps imagemagick python3 dbus-x11 fonts-dejavu-core \
      libegl1 libgl1 libgles2 file git libgtk-3-0t64 \
    && rm -rf /var/lib/apt/lists/*

# 0005. mdns-sd for finding peers

- **Status:** Accepted
- **Date:** 2026-10-07

## Context

Peers on the same network must find each other with no server and no setup.

## Decision

Announce and browse a `_takkie._udp` service with `mdns-sd`, with a manual
peer list as a fallback for networks that block multicast.

## Consequences

- Pure Rust, works on all three desktop systems and on Android once the app
  holds a Wi-Fi multicast lock.
- Guest Wi-Fi and "AP isolation" can still block it, hence the manual peers.

## Reconsider if

We build the iOS app: Apple requires a special multicast entitlement, so we
may need NWBrowser through a Swift plugin there.

Alternatives considered: libmdns, Bonjour or NSD through platform plugins.

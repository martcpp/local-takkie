# Real-Time Audio Streaming Guide

This document explains how to use the real-time audio streaming feature in VideoLAN.

## Overview

The application now supports two modes:
1. **Push-to-Talk (PTT)** - Default mode for text-based communication
2. **Audio Streaming** - Real-time audio streaming mode for voice communication

## Running Audio Streaming Mode

### Basic Command

```bash
vl <instance_name> <port> audio
```

### Parameters

- `<instance_name>` - Your user/device name (e.g., "Alice", "Bob")
- `<port>` - UDP port to use (e.g., 5000, 5001)
- `audio` - Mode flag to enable audio streaming

### Examples

**User 1 - Streaming Audio:**
```bash
vl Alice 5000 audio
```

**User 2 - Receiving Audio:**
```bash
vl Bob 5001 audio
```

## How It Works

### Audio Capture & Send
1. Application captures audio from your system's default microphone
2. Audio samples are encoded and packaged into UDP packets
3. Packets are sent to all discovered peers on the network
4. Each packet starts with a marker byte (0x01) to identify audio data

### Audio Receive & Playback
1. Application listens for incoming UDP packets on the specified port
2. Audio packets are received and processed
3. Audio is sent to the system's default speaker for real-time playback

## Network Requirements

- Devices must be on the same local network for mDNS discovery
- UDP ports specified must be available and not blocked by firewalls
- Both devices should use different ports to avoid conflicts

## Features

✅ Real-time audio streaming over UDP
✅ Automatic peer discovery via mDNS
✅ Multi-peer support (audio is broadcast to all discovered peers)
✅ Simultaneous send and receive
✅ CLI-based control

## Technical Details

### Audio Configuration
- Uses CPAL (Cross-Platform Audio Library) for audio I/O
- Default system audio input device for microphone capture
- Default system audio output device for speaker playback
- Audio samples are 32-bit floating-point

### Packet Format
- Marker byte: `0x01` (audio packet identifier)
- Payload: Raw audio sample data in little-endian format
- Maximum packet size: 4096 bytes

### Threading
- Audio capture runs in a separate thread
- Audio playback runs in a separate thread
- UDP transmission happens asynchronously
- Main thread remains available for control

## Troubleshooting

### No Audio Output
- Check that your system audio output device is working
- Verify the output device is not muted
- Check system volume settings

### No Audio Input
- Check that your microphone is connected and working
- Verify microphone permissions in system settings
- Check system volume settings for input device

### Network Issues
- Ensure devices are on the same local network
- Check that UDP ports are not blocked by firewall
- Verify mDNS discovery is working (check for "Found new peer" messages)

### Cannot Bind Port
- Ensure the specified port is not already in use
- Try using a different port number
- Check system firewall rules for UDP

## Switching Modes

### From Audio Streaming to Push-to-Talk
```bash
# Stop current process (Ctrl+C)
# Run in PTT mode (default)
vl Alice 5000
```

### From Push-to-Talk to Audio Streaming
```bash
# Stop current process (Ctrl+C)
# Run in audio mode
vl Alice 5000 audio
```

## Performance Tips

1. Use lower audio sample rates if experiencing latency
2. Reduce audio buffer sizes for lower latency (trade-off: may increase CPU usage)
3. Ensure network bandwidth is available for audio transmission
4. Close other applications that use audio to avoid conflicts
5. Use wired network connections for more stable audio streaming

## Future Improvements

- Audio compression to reduce bandwidth
- Adjustable audio quality/bitrate settings
- Recording audio streams to file
- Audio file playback capability
- Echo cancellation and noise reduction
- Volume level monitoring and control

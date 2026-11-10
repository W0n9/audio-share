#!/usr/bin/env python3
"""Generate a deterministic stereo sine WAV for PipeWire null-sink tests."""

import argparse
import math
import struct
import wave


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output")
    parser.add_argument("--frequency", type=float, default=440.0)
    parser.add_argument("--sample-rate", type=int, default=48000)
    parser.add_argument("--channels", type=int, default=2)
    parser.add_argument("--duration", type=float, default=10.0)
    parser.add_argument("--amplitude", type=float, default=0.25)
    args = parser.parse_args()

    if args.channels not in (1, 2):
        parser.error("--channels must be 1 or 2")
    frames = int(args.sample_rate * args.duration)
    peak = int(32767 * args.amplitude)

    with wave.open(args.output, "wb") as output:
        output.setnchannels(args.channels)
        output.setsampwidth(2)
        output.setframerate(args.sample_rate)
        block = bytearray()
        for index in range(frames):
            value = int(
                peak * math.sin(2.0 * math.pi * args.frequency * index / args.sample_rate)
            )
            sample = struct.pack("<h", value)
            block += sample * args.channels
        output.writeframes(bytes(block))

    print(args.output)


if __name__ == "__main__":
    main()

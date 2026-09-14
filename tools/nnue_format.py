"""Strict Jakgro NNUE v1 serialization and a dependency-free integer oracle.

This format is not compatible with Stockfish networks. FNV-1a detects accidental
corruption, not authenticity; artifact identity is recorded separately by SHA-256.
"""

from __future__ import annotations

from array import array
from dataclasses import dataclass
import math
import struct
import sys

INPUTS = 12_288
HIDDEN = 128
ACTIVATION = 255
OUTPUT_SCALE = 64
CP_DIVISOR = ACTIVATION * OUTPUT_SCALE
MAX_SCORE = 16_000
PAYLOAD_BYTES = 3_146_500
FILE_BYTES = 3_146_548
HEADER = struct.Struct("<8s8IQ")
FIELDS = (b"JAKNNUE\0", 1, 1, INPUTS, HIDDEN, ACTIVATION, OUTPUT_SCALE, PAYLOAD_BYTES, 0)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def checksum(payload: bytes) -> int:
    value = 14_695_981_039_346_656_037
    for byte in payload:
        value = ((value ^ byte) * 1_099_511_628_211) & 0xFFFF_FFFF_FFFF_FFFF
    return value


def round_away(value: float) -> int:
    """Round halfway values away from zero, matching Rust float::round."""
    require(math.isfinite(value), "cannot quantize a nonfinite value")
    return math.floor(value + 0.5) if value >= 0 else math.ceil(value - 0.5)


def trunc_div(numerator: int, denominator: int) -> int:
    require(denominator > 0, "division scale must be positive")
    return numerator // denominator if numerator >= 0 else -((-numerator) // denominator)


def features(values) -> tuple[int, ...]:
    result = tuple(values)
    require(2 <= len(result) <= 64, "a perspective requires 2..64 active features")
    require(all(type(value) is int and 0 <= value < INPUTS for value in result), "invalid feature index")
    require(all(a < b for a, b in zip(result, result[1:])), "features must be sorted and unique")
    require(len({value // 768 for value in result}) == 1, "mixed king buckets")
    return result


def i16_bytes(values) -> bytes:
    result = array("h")
    require(result.itemsize == 2, "this platform has no 16-bit short array")
    for value in values:
        require(type(value) is int and -32768 <= value <= 32767, "weight is outside signed i16")
        result.append(value)
    if sys.byteorder != "little":
        result.byteswap()
    return result.tobytes()


@dataclass(frozen=True)
class Network:
    hidden_bias: tuple[int, ...]
    input_weights: array
    output_weights: tuple[int, ...]
    output_bias: int

    def validate(self) -> None:
        require(len(self.hidden_bias) == HIDDEN, "wrong hidden bias shape")
        require(len(self.input_weights) == INPUTS * HIDDEN, "wrong feature transformer shape")
        require(len(self.output_weights) == 2 * HIDDEN, "wrong output shape")
        require(self.input_weights.typecode == "h" and self.input_weights.itemsize == 2,
                "feature weights must be an i16 array")
        require(all(type(value) is int and -32768 <= value <= 32767
                    for value in (*self.hidden_bias, *self.output_weights)), "weight outside signed i16")
        require(type(self.output_bias) is int and -(1 << 31) <= self.output_bias < (1 << 31),
                "output bias outside signed i32")
        bound = abs(self.output_bias) + ACTIVATION * sum(abs(value) for value in self.output_weights)
        require(bound <= (1 << 31) - 1, "output arithmetic exceeds signed i32")

    def to_bytes(self) -> bytes:
        self.validate()
        inputs = array("h", self.input_weights)
        if sys.byteorder != "little":
            inputs.byteswap()
        payload = (i16_bytes(self.hidden_bias) + inputs.tobytes() + i16_bytes(self.output_weights)
                   + struct.pack("<i", self.output_bias))
        require(len(payload) == PAYLOAD_BYTES, "payload length mismatch")
        return HEADER.pack(*FIELDS, checksum(payload)) + payload

    @classmethod
    def from_bytes(cls, blob: bytes) -> Network:
        require(len(blob) == FILE_BYTES, "wrong network file length")
        header = HEADER.unpack_from(blob)
        require(header[:-1] == FIELDS, "unsupported network header")
        payload = blob[HEADER.size:]
        require(checksum(payload) == header[-1], "network checksum mismatch")
        hidden = struct.unpack_from(f"<{HIDDEN}h", payload)
        end = 2 * (HIDDEN + INPUTS * HIDDEN)
        inputs = array("h")
        inputs.frombytes(payload[2 * HIDDEN:end])
        require(inputs.itemsize == 2, "this platform has no 16-bit short array")
        if sys.byteorder != "little":
            inputs.byteswap()
        output = struct.unpack_from(f"<{2 * HIDDEN}h", payload, end)
        bias = struct.unpack_from("<i", payload, PAYLOAD_BYTES - 4)[0]
        network = cls(hidden, inputs, output, bias)
        network.validate()
        return network

    def infer(self, white, black, side_to_move: int) -> dict:
        require(type(side_to_move) is int and side_to_move in (0, 1), "invalid side to move")
        sums = []
        for indices in (features(white), features(black)):
            total = list(self.hidden_bias)
            for index in indices:
                offset = index * HIDDEN
                for unit in range(HIDDEN):
                    total[unit] += self.input_weights[offset + unit]
            sums.append(total)
        ordered = sums[side_to_move] + sums[1 - side_to_move]
        numerator = self.output_bias + sum(weight * min(ACTIVATION, max(0, value))
                                           for weight, value in zip(self.output_weights, ordered))
        score = max(-MAX_SCORE, min(MAX_SCORE, trunc_div(numerator, CP_DIVISOR)))
        return {"cp": score, "numerator": numerator, "white": sums[0], "black": sums[1]}

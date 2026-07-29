# Vivid Protocol 1.5 Canvas Surface

This file is a normative part of the
[Vivid Protocol 1.5 specification](vivid-protocol-1.5-spec.md) and defines
`canvas-surface-v1`.

## 1. Scope

`canvas-surface-v1` is a terminal-free presentation target with one logical viewport and an
explicit mapping to physical pixels. It is suitable for browser canvases, application panels,
off-screen render targets, and nested presenter roots that do not need desktop output topology.

It does not define desktop input. A deployment needing OS desktop injection selects
`desktop-surface-v1` instead.

## 2. Target descriptor

For this profile, `WELCOME` target descriptor key 5 contains:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Logical viewport width, nonzero |
| 1 | uint | Logical viewport height, nonzero |
| 2 | uint | Physical pixel width, nonzero |
| 3 | uint | Physical pixel height, nonzero |
| 4 | uint | Scale numerator, nonzero |
| 5 | uint | Scale denominator, nonzero |
| 6 | uint | Clockwise rotation: `0`, `90`, `180`, `270` |
| 7 | bool | Geometry is settled |
| 8 | uint | Color space; sRGB (`1`) |

The scale is descriptive of the intended logical-to-physical transform. The exact final mapping
uses logical and physical dimensions, rotation, and presenter clipping with checked arithmetic.

`TARGET_CHANGED` carries the complete descriptor, new target generation, and reason mask:
logical size (`0`), physical backing size (`1`), scale (`2`), rotation (`3`), target recreation
(`4`), and visibility/attachment (`5`).

Continuous resize is coalesced to at most one event per compositor frame per source of truth. A
final settled event is delivered when the presenter has final geometry.

## 3. Canvas content surfaces

A producer may create semantic profile `canvas-content-v1` with:

- canvas logical-unit coordinate model (`3`); or
- normalized 32.32 coordinate model (`2`).

Its profile-specific map is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Producer-defined logical-unit scale numerator |
| 1 | uint | Producer-defined logical-unit scale denominator |
| 2 | uint | Semantic canvas generation |
| 3 | uint | Intended color space; sRGB (`1`) |

Scale values are nonzero. A logical size, scale, rotation, or semantic-generation change advances
`surface_generation`. Track replacement does not.

## 4. Canvas node geometry

The node geometry map is:

| Key | Type | Meaning |
|---:|---|---|
| 0 | uint | Coordinate space: target logical units (`1`) or normalized target (`2`) |
| 1 | int | X in signed 32.32 |
| 2 | int | Y in signed 32.32 |
| 3 | int | Positive width in signed 32.32 |
| 4 | int | Positive height in signed 32.32 |

For normalized target coordinates, `0` is the target origin and `1 << 32` is the complete width or
height. For logical units, integer one is one target logical unit.

The optional clip map has x, y, width, and height in the same coordinate space. Fit occurs before
clip. Final rendering clips to the physical viewport.

A commit names the expected target generation. The presenter never silently applies geometry
computed for an older logical or physical viewport.

## 5. Nested presentation

A nested presenter typically selects `canvas-surface-v1` for its inner target and represents that
target as one stable outer surface. It:

- keeps inner and outer target/surface generations independent;
- converts geometry with checked arithmetic;
- never presents an inner scene activation as proof of outer presentation;
- re-originates track channels and flow limits; and
- reports `NOT_VISIBLE` to an inner presentation wait when the outer canvas is not projected.

## 6. Canvas conformance

A conforming presenter tests fractional scale, rotation, zero/maximum dimensions, resize
coalescing, stale target generations, normalized clipping, and two nested canvases reusing local
surface/node IDs. No grid, cell, text-layer, anchor, output-topology, or desktop-input field is
required or fabricated.

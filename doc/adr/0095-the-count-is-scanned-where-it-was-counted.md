# 0095 — The count is scanned where it was counted, and the host stops waiting

Date: 2026-08-27. Status: **accepted, and built** — stage B in the one shape ADR
0091 and ADR 0092 left standing. Their findings, restated as constraints: the
kernels' costs move together, so a scheduling change must remove the sync *and* the
sparsity at once (ADR 0092); the exact count is not negotiable and a bound is not a
count (ADR 0084 §1, re-confirmed twice); and a refusal keeps its name (principle 3).

## The shape

The lane's chain becomes **one submission with no mid-frame readback**:

1. **count** — the exact pass, unchanged: the subdivision is the price of exactness
   and it is paid where it is cheap to pay, on the device.
2. **scan** — an exclusive prefix over the per-tile counts, computed **on the
   device** into the offsets buffer (with the total appended). At fifty-eight
   thousand tiles this is tens of microseconds of serial adds in one workgroup;
   nothing about it earns a multi-block scan.
3. **emit** — reads its offsets from the scan, never from the host, and writes into
   an **edges buffer of persistent, grow-only capacity**. Every write is guarded by
   the capacity; an overflow stops writing, keeps counting, and raises a flag.
4. **deposit** — unchanged, and *dense*: the offsets are exact, so the sparsity that
   sank ADR 0092's bound scheme never exists.

The host's part of the old chain — submit the count, **stall on its map**, prefix on
the CPU, allocate exactly, submit the rest — becomes: submit everything, and read
**eight bytes** (the flag and the total) after the wait the frame already pays at its
end, before anything is presented.

## Exactness, kept — and where it moved

The old chain allocated exactly per frame and refused before allocating. This chain
holds the same two promises in a different order:

- **No wrong picture, ever.** The flag is read before the present. A frame whose
  emit met its capacity is not presented; the capacity grows to the scanned total
  and the frame **re-runs whole** — dispatch and content pass — then presents. The
  redo is the first frame's ordinary cost (capacity starts from a heuristic and the
  first overflow sizes it) and a rare growth event afterwards; a steady zoom pays
  zero.
- **The refusal keeps its name.** A total past `max_frame_bytes` refuses with both
  numbers, exactly as before — one frame later in wall-clock terms than the old
  refusal, and still before any pixel of it exists anywhere a person can see.
- **The budget prices capacity.** The persistent buffers are charged at their
  capacity against the same frame budget as the per-frame allocations they replace;
  growth is clamped by it, so the high-water can never exceed what the old chain
  could have allocated in one frame.

Determinism is untouched by construction: the count, the emit's edge content and
order, and the deposit's arithmetic are all bit-for-bit the shipped ones — only
*where the offsets are added up* moved, and a prefix sum has one answer in integers.

## What it buys, measured

(Numbers recorded beside the build, on the worst page, 890M.) The old chain's host
timeline per zoom step: ~10–19 ms stalled on the count's map, then the frame-end
wait for emit+deposit+content. The new chain's host timeline: the frame-end wait
alone. The GPU serial work is unchanged, so the step approaches the kernels' own
floor — which ADR 0091 named as the wall and stage B was never going to lower; what
it removes is everything the host used to add on top.

## Held by

Every existing compute gate unmodified — the pixels are the same bytes — plus the
growth road forced from a one-byte starting capacity and held to the steady road's
bytes, and the budget refusal asserted at its new site with its old name.

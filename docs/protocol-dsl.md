<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Protocol DSL

The version 1 interpreter accepts only declared write, bounded read, byte expectation, and bounded delay operations. The core owns transport handles and cancellation. Default limits are 64 steps, 1,024 bytes per report/buffer, 16 reads, and 2 seconds total delay. There is no expression evaluator, filesystem, network, process, recursion, arbitrary loop, or system call.

Future checksum, field, fragmentation, forwarding, snapshot, read-modify-write, and finite-state-machine operations must remain typed, versioned, bounded, and Fixture-testable. A missing operation is a capability gap, never permission to execute plugin code.


# `sys.*` Architecture — Design for V1.1.0

**Status:** design only. **Nothing in this document is implemented.**
**Edition:** ASCENSION +A Core V1.1.0

Task scope: design a safe system-service architecture — capabilities, a
privilege boundary, and runtime services — so that "+A Foundation" can be built
on it, **without** building a kernel and **without** weakening the isolation of
the unprivileged AVM.

## 1. Current state (as implemented)

| Element | Implemented | Behaviour |
|---|---|---|
| Opcodes `sys.*` (11) | yes | decode, verify, dispatch |
| Capability declaration in images | yes | `CAPS` section |
| Verifier requirement | yes | an image using `sys.*` must declare the matching bit |
| Privilege flag on the machine | yes | `Avm::privileged`, default `false` |
| Capability bitmask on the machine | yes | `Avm::capabilities`, default `0` |
| Host service implementations | **no** | every service returns `UNSUPPORTED_SERVICE` |

So the *shape* of the boundary exists and is enforced, but nothing is reachable
through it. This is the correct resting point for Core: it makes the security
question already answered ("can a program reach the host? no") while leaving the
service table empty.

## 2. Design goals

1. **Default deny.** An unprivileged machine, or a privileged machine without
   the capability, cannot invoke any service. The check happens before any
   service-specific code runs.
2. **No ambient authority.** A program holds no handles it did not ask for. Every
   service takes explicit arguments; nothing is implicit in the program's
   identity.
3. **The verifier and the runtime agree.** If an image declares a capability it
   does not use, that is harmless; if it uses one it does not declare, the image
   is rejected *before* execution, not at first call.
4. **Uniform failure.** A service that is absent, ungranted, or malformed fails
   with a distinguishable error — never a silent no-op, never host I/O.
5. **Deterministic.** The same image with the same grants behaves identically in
   the interpreter and in a hosted native run.
6. **No kernel.** No preemption, no address spaces, no device drivers, no
   scheduler. Those are Foundation's job. Core only defines the gate.

## 3. Layering

```
        +A program (bytecode, unprivileged)
                  │  sys.<service>  (opcode)
                  ▼
        ┌───────────────────────────┐
        │ AVM privilege gate        │  privileged?  capability?   ← Core owns this
        └───────────────────────────┘
                  │  ServiceRequest (validated, typed)
                  ▼
        ┌───────────────────────────┐
        │ Service table (host side) │  ← graft point; empty in Core
        └───────────────────────────┘
                  │
                  ▼
        host: files, devices, memory, other machines
```

The gate is the **entire** Core contribution. Everything below the gate is
supplied by an embedder; Core defines its interface and its failure modes.

## 4. Capabilities

Capabilities are a fixed-width bitmask so they fit the `CAPS` section and the
verifier's existing check.

| Bit | Name | Authority granted | Services |
|---|---|---|---|
| 0 | `CAP_MEMORY_MAP` | Create/destroy regions in the *requesting* machine | `sys.region.map`, `sys.region.unmap` |
| 1 | `CAP_IO_PORT` | Read/write a port the host exposes | `sys.port.read`, `sys.port.write` |
| 2 | `CAP_IRQ_MANAGE` | Bind/enable/disable an interrupt line | `sys.irq.bind`, `sys.irq.enable`, `sys.irq.disable` |
| 3 | `CAP_CONTEXT_SWITCH` | Enter/leave a kernel-managed context | `sys.context.enter`, `sys.context.return` |
| 4 | `CAP_CPU_CONTROL` | Halt, reset, or change CPU mode | `sys.cpu.control` |
| 5 | `CAP_POWER_CONTROL` | Power off, sleep, wake | `sys.power` |
| 6–31 | reserved | must be zero | — |

### 4.1 Properties of the capability model

* **Monotonic within a machine.** Capabilities are granted when the machine is
  constructed and are never escalated by guest code. There is no `sys.grant`.
* **Additive, never interpreted as hierarchy.** Holding bit 0 says nothing about
  bit 1. `CAP_CPU_CONTROL` does not imply the power bit.
* **Two-person rule for dangerous bits.** `CAP_CPU_CONTROL` and
  `CAP_POWER_CONTROL` should require the embedder to opt in explicitly *and*
  record the decision; the design recommends they be denied by default in every
  future profile.
* **Declared in the image, granted by the host.** The `CAPS` section is a
  *request*, verified for consistency. The host decides what to grant. A host may
  grant less than requested; a call to an ungranted service fails with
  `PRIVILEGE_VIOLATION`.

## 5. The privilege boundary

The boundary is a single predicate, evaluated in the interpreter before
dispatching any `sys.*` opcode:

```
invoke(service):
    if not machine.privileged:              → PRIVILEGE_VIOLATION
    if machine.capabilities & bit(service) == 0:
                                           → PRIVILEGE_VIOLATION
    if service_table[service] is absent:    → UNSUPPORTED_SERVICE
    return service_table[service](request)  // typed, validated
```

Properties:

* **The guest never supplies a host pointer or a host file descriptor.** All
  arguments are guest values already validated by the verifier (registers,
  immediates, memory operands). A service that needs host resources receives
  them from the table, not from the guest.
* **No service can return a capability.** Results are plain 64-bit values.
* **Errors do not leak host detail.** A denied or absent service yields the same
  two error codes regardless of *why* the host declined.
* **Native tiers inherit the gate.** The native backends do not lower any
  `sys.*` opcode ([JIT-AOT.md](JIT-AOT.md)), so a native program cannot reach a
  service at all. This is intentional: it makes the native tiers strictly *less*
  privileged than the interpreter, never more.

## 6. Service surface

Each service is specified as: arguments (in which registers by convention),
result, capability, and failure modes. Argument placement is a **convention**
that the eventual Foundation ABI must fix; the point of this table is to state
what may be exposed, to whom, and how it fails.

| Service | Capability | Request | Result / effect | Failure modes |
|---|---|---|---|---|
| `sys.region.map` | 0 | base, length, permissions | a new region owned by this machine | length 0; overlaps existing; exceeds host quota → `MEMORY_ACCESS` |
| `sys.region.unmap` | 0 | base | region destroyed | unknown base → `MEMORY_ACCESS` |
| `sys.port.read` | 1 | port number | port value | port not exposed by host → `MEMORY_ACCESS` |
| `sys.port.write` | 1 | port number, value | none | port not exposed → `MEMORY_ACCESS` |
| `sys.irq.bind` | 2 | line, block ID | handler associated | line not exposed → `INVALID_OPERAND` |
| `sys.irq.enable` / `.disable` | 2 | line | line state | line not bound → `INVALID_OPERAND` |
| `sys.context.enter` / `.return` | 3 | context ID | machine state switched | unknown context → `INVALID_OPERAND` |
| `sys.cpu.control` | 4 | mode | mode change | mode unsupported → `UNSUPPORTED_SERVICE` |
| `sys.power` | 5 | action | action performed | action unsupported → `UNSUPPORTED_SERVICE` |

Deliberately **absent** from the surface, and recommended to stay absent:

* any service that opens a path, socket, or process;
* any service that maps host memory into the guest address space implicitly —
  mapping is *guest* memory, allocated by the host, never a host pointer handed
  back;
* any service that lets a guest name another machine;
* any service that reads or writes the image loader's state.

## 7. Resource accounting (design requirement)

Foundation will need limits; Core should not implement them, but the interface
must be able to express them:

* Each machine should carry a region quota and a total-bytes quota, so
  `sys.region.map` can refuse rather than exhaust host memory.
* The service table should be able to report "quota exceeded" distinctly from
  "not permitted", so a program can react without probing.
* Core's part is to leave room for those error variants rather than collapsing
  everything into one code.

## 8. Why not implement it now

1. **A service table is a kernel seed.** Implementing `sys.region.map` for real
   requires an allocator, an ownership model, and a lifetime rule — that is
   Foundation.
2. **Isolation is a feature.** Today an AVM program provably cannot touch the
   host: there is no code path to do it. Implementing services converts a proof
   into a set of checks that must be tested and audited.
3. **The gate is testable without services.** `every_system_service_requires_a_capability`
   already proves that all 11 services are refused unprivileged, refused without
   the bit, and refused as unsupported when both are present. When Foundation
   fills the table, those tests keep the boundary honest.

## 9. Migration path to Foundation

1. Add `ServiceTable` to the embedder API: `type Service = fn(&mut Machine, Request)
   -> Result<Value, ServiceError>`.
2. Implement `sys.region.map`/`unmap` against a host-owned allocator, keeping the
   two-stage gate unchanged.
3. Add quotas and the distinct "quota exceeded" error.
4. Only then consider ports, IRQs, and contexts, each behind its own bit and its
   own profile.
5. Keep the native tiers `sys.*`-free unless a documented, deny-by-default
   native service model exists.

## 10. Summary

Core ships an **enforced but empty** privilege boundary: capability bits are
declared in images, checked by the verifier, and re-checked at run time before
dispatch; every service currently returns `UNSUPPORTED_SERVICE`. Foundation can
fill the service table without changing a single byte of the boundary logic, and
without ever having to relax the unprivileged machine.

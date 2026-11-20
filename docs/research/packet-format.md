# Research: Packet Payload Formats

**English** | [한국어](packet-format.ko.md)

- Date: 2026-11-17
- Purpose: Compare options for decision N4 in `02-session-network.md`
- Result: **Adopted Protobuf + `prost`** (2026-11-17). There will be no Godot web export, so GDScript code generation isn't needed
- Scope: The **payload** of client ↔ server game packets. Header framing (`len u32 | msg u16 | flags u16`) and server-to-server control messages are already defined and are outside this scope

## 1. Requirements

| # | Requirement | Reason |
|---|---|---|
| R1 | **Generate** Rust, C# (Unity, Godot .NET), C++ (Unreal), and GDScript (Godot) code from one schema | Handwritten serialization for each engine will inevitably diverge |
| R2 | Remain compatible with older clients when fields are added or removed | Minimize forced client updates during live service |
| R3 | Keep allocations and copies low for frequent small packets (movement, combat) | 10,000 concurrent users at 10 Hz |
| R4 | Use a mature, safe Rust implementation | The guide prohibits `unsafe`, and a panic means process exit |
| R5 | Make build integration easy for each engine | Especially Unreal, where external C++ libraries are hard to integrate |

### Why Godot Needs GDScript Support

Godot 4 C# projects **can't be exported to the web.** The official documentation says, "Projects written in C# using Godot 4 currently cannot be exported to the web". A Godot web client therefore needs GDScript code generation. Without plans for a web release, Godot can use C# as well, which makes R1 much easier to satisfy.

## 2. Support by Candidate

| | Rust (server) | Unity (C#) | Unreal (C++) | Godot GDScript | Godot C# |
|---|---|---|---|---|---|
| **Protobuf** | `prost` (tokio ecosystem, de facto standard) + Google's official `protobuf` v4 crate (released 2025-06) | Official `Google.Protobuf` (may need `link.xml` to prevent IL2CPP code stripping) | No official support. Integrate libprotobuf as a third-party module. Several community plugins exist | Community `godobuf` (listed in the Asset Library) | Official `Google.Protobuf` |
| **FlatBuffers** | Official crate. Several past advisories (RUSTSEC, 2019 to 2021) reported soundness issues that caused memory errors in safe code | Official | Official (header-only, so easy to integrate) | Community (two implementations: pure GDScript and GDExtension) | Official C# |
| **Custom IDL + generator** | Generate in-house | Generate in-house | Generate in-house (can emit engine types `FString`, `TArray`, and `USTRUCT` directly) | Generate in-house | Generate in-house |

## 3. Evaluation by Candidate

### 3.1 Protobuf (proto3)

- **Pros**
  - It has the broadest language and tooling support. All four targets have real-world use cases.
  - The format has R2 (compatibility) built in. As long as field numbers are respected, older versions ignore new fields.
  - Rust's `prost` is mature and safe. It generates plain Rust structs that are easy to read.
- **Cons**
  - Decoding creates message objects. In C#, this adds GC overhead. Reusing messages (`MergeFrom`) can reduce it.
  - Unreal integration takes the most work. It requires platform-specific libprotobuf builds and handling conflicts with engine macros, so developers usually use a plugin.
  - The generated types aren't native to the engine. Exposing them to Unreal Blueprint requires a conversion layer.

### 3.2 FlatBuffers

- **Pros**
  - Read directly from the buffer without deserialization (zero-copy). This works well for large data that's mostly read.
  - Unreal integration is easy.
- **Cons**
  - The write API (builder) is cumbersome. It slows development in game server code that frequently creates small packets.
  - The Rust crate's history of soundness issues raises concerns for R4. Current versions expect a verifier to validate the buffer before reading it.
  - GDScript support comes only from the community.
  - Zero-copy offers little benefit for small packets. The format works best for large static data such as **design data tables**.

### 3.3 Custom IDL + Custom Code Generator

This approach is common among Korean game companies. Define the schema in a custom format (TOML, YAML, or a dedicated syntax), and use one Rust generator to emit code in all four languages.

- **Pros**
  - Full control. The wire format is a binary format you define, so experience with C++ engines carries over directly.
  - The generator can emit native engine types directly. Examples include Unreal `USTRUCT`/Blueprint and Godot `Dictionary`/`Resource`. This is the easiest option for client developers to use.
  - There are no external library dependencies, which eliminates the Unreal integration problem.
- **Cons**
  - You **maintain all four generators** (Rust, C#, C++, GDScript) and their respective runtimes yourself.
  - You have to design the compatibility rules (R2) yourself. For example, use field numbers or tags and length prefixes so readers can skip unknown fields.
  - The upfront cost is high. It becomes a large side effort for a project whose purpose is to learn server design.

### 3.4 Excluded

- **MessagePack, JSON**: They lack a schema, so they don't meet R1 (code generation).
- **Cap'n Proto**: No GDScript implementation was found.

## 4. Recommendation

**First choice: Protobuf (proto3) + Rust `prost`**

- It meets R1 through R4. For R5, only Unreal is difficult, and a plugin can handle it.
- This project focuses on server design and learning Rust. Don't spend time building serialization tools.
- The architecture keeps replacement costs low. Only the `protocol` crate interprets payloads (document 02, section 8.2). Switching to a custom format later leaves the Gateway, envelope, and game logic unchanged.

**Second choice: Custom IDL + generator.** Consider switching if Unreal becomes the main client and ease of use in Blueprint matters. A compromise is to keep `.proto` files as the source schema and generate only the Unreal wrappers in-house.

**Keep FlatBuffers as a separate candidate for design data tables only**, outside packet serialization.

## 5. Rules for Using Protobuf (Draft)

- Keep schema files in a single `proto/` directory. The server and all clients use it as the source of truth.
- Generate the mapping from message IDs (`msg u16`) to message types from the schema as well. Don't maintain it by hand.
- Don't reuse field numbers. Mark deleted fields as `reserved`.
- Keep frequent messages such as movement updates to as few fields as possible, and quantize coordinates to integers (for example, `sint32` in cm).
- Immediately after decoding, the server's `protocol` crate converts types generated by `prost` to **validated domain types** (guide section 7, "Validate at the boundary"). Keep generated types out of the `game` crate.

## Sources

- Godot 4 C# web export is unsupported: https://docs.godotengine.org/en/latest/tutorials/export/exporting_for_web.html
- Google's official Rust protobuf crate: https://crates.io/crates/protobuf/4.33.4-release , https://groups.google.com/g/protobuf/c/Ur70AGjgHnw
- Benchmark discussion for prost and Google protobuf: https://github.com/tokio-rs/prost/issues/1469
- GDScript Protobuf (godobuf): https://github.com/oniksan/godobuf , https://godotengine.org/asset-library/asset/3734
- GDScript FlatBuffers: https://github.com/splatterfacegames/godot-flatbuffers , https://github.com/enetheru/godot-flatbuffers
- Unreal Protobuf plugin examples: https://github.com/DarkestLink-Dev/UE-LinkProtobuf , https://github.com/JGameEngine/UE-Protobuf-Plugin
- FlatBuffers Rust soundness advisories: https://rustsec.org/advisories/RUSTSEC-2021-0122.html , https://rustsec.org/advisories/RUSTSEC-2020-0009.html
- Unity IL2CPP stripping: https://issuetracker.unity3d.com/issues/ios-link-dot-xml-does-not-prevent-il2cpp-stripping-of-generic-classs-constructor-in-a-library

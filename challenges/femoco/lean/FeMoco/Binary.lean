import FeMoco.Controller

/-! Strict FEMOOPS1 decoding for the signed-basis subset. The decoder itself is
in Lean: a generated theorem can bind a certificate to the complete byte list,
not merely to an exporter-supplied program and hash. Unsupported operations fail.
-/
namespace FeMoco.Binary
open Controller Boolean

def absent : Nat := 2^64-1

def little (bytes : List Nat) : Nat :=
  match bytes with
  | [] => 0
  | b :: bs => b + 256 * little bs

structure Record where
  kind : Nat
  pad : Nat
  c2 : Nat
  c1 : Nat
  target : Nat
  bit : Nat
  condition : Nat
  reg : Nat
  deriving Repr

def record (b : List Nat) : Record :=
  ⟨little (b.take 4), little ((b.drop 4).take 4),
    little ((b.drop 8).take 8), little ((b.drop 16).take 8),
    little ((b.drop 24).take 8), little ((b.drop 32).take 8),
    little ((b.drop 40).take 8), little ((b.drop 48).take 8)⟩

def wire (n q : Nat) : Option (Fin n) := if h : q < n then some ⟨q,h⟩ else none

/-- Classical bits can refer only to an earlier unconditional HMR. Reusing a bit
updates its binding; distinct HMR occurrences always get fresh outcome variables. -/
def decodeRecord (n next : Nat) (bits : Nat → Option Nat) (r : Record) :
    Option (Gate n × Nat × (Nat → Option Nat)) := do
  if r.pad != 0 || r.reg != absent then none else do
  let g ← match r.kind with
    | 0 => if [r.c2,r.c1,r.target,r.bit].all (· == absent) then some .neg else none
    | 6 | 7 | 11 => do
      if ![r.c2,r.c1,r.bit].all (· == absent) then none else do
      let t ← wire n r.target
      pure (if r.kind == 6 then .x t else if r.kind == 7 then .z t else .reset t)
    | 8 | 9 => do
      if ![r.c2,r.bit].all (· == absent) then none else do
      let c ← wire n r.c1
      let t ← wire n r.target
      pure (if r.kind == 8 then .cx c t else .cz c t)
    | 13 | 14 => do
      if r.bit != absent then none else do
      let a ← wire n r.c2
      let b ← wire n r.c1
      let t ← wire n r.target
      pure (if r.kind == 13 then .ccx a b t else .ccz a b t)
    | 12 => do
      if ![r.c2,r.c1,r.condition].all (· == absent) || r.bit ≥ 2^32-1 then none else do
      let t ← wire n r.target
      pure (.hmr t next)
    | _ => none
  if !(decide (proper g)) then none else do
  if r.kind == 11 && r.condition != absent then none else do
  if r.kind == 12 then
    pure (g, next+1, fun b => if b = r.bit then some next else bits b)
  else if r.condition == absent then pure (g,next,bits)
  else do
    let outcome ← bits r.condition
    pure (.whenOutcome outcome g,next,bits)

def records (n : Nat) : Nat → Nat → (Nat → Option Nat) → List Nat → Option (List (Gate n))
  | 0, _, _, [] => some []
  | 0, _, _, _ => none
  | k+1, next, bits, bytes => do
    if bytes.length < 56 then none else do
    let (g,next',bits') ← decodeRecord n next bits (record (bytes.take 56))
    let rest ← records n k next' bits' (bytes.drop 56)
    pure (g :: rest)

/-- Bytes are natural literals so even malformed byte values can be rejected.
The cap is an explicit refusal, not a partial certificate. -/
def decode (n : Nat) (bytes : List Nat) : Option (List (Gate n)) := do
  if n == 0 || n > 4096 || !bytes.all (· < 256) then none else do
  if bytes.take 8 != [70,69,77,79,79,80,83,49] || bytes.length < 16 then none else do
  let count := little ((bytes.drop 8).take 8)
  if count > 100000 || bytes.length != 16 + count*56 then none else do
  records n count (n+1) (fun _ => none) (bytes.drop 16)

/-- Zero workspace, independent data inputs, and an arbitrary incoming sign.
Wire inputs have labels < n, sign has label n, HMR labels begin at n+1. -/
def initial (n : Nat) (inputs : List (Fin n)) : Symbolic n :=
  ⟨fun w => if w ∈ inputs then atom w.val else [], atom n, []⟩

/-- An identity-controller certificate bound to every literal input byte.
This is not a certificate for a molecular system operator. -/
structure Certificate (bytes : List Nat) (n : Nat) (inputs : List (Fin n))
    (bounds : Bounds) where
  program : List (Gate n)
  decoded : decode n bytes = some program
  controller : Controller.Certificate program (initial n inputs) (initial n inputs) bounds

 theorem Certificate.correct {bytes n inputs bounds} (h : Certificate bytes n inputs bounds)
    (v : Assignment) :
    (runConcrete v h.program (evaluate v (initial n inputs))).wires =
      (evaluate v (initial n inputs)).wires ∧
    (runConcrete v h.program (evaluate v (initial n inputs))).phase =
      (evaluate v (initial n inputs)).phase ∧
    (runConcrete v h.program (evaluate v (initial n inputs))).badReset = false :=
  h.controller.correct v

end FeMoco.Binary

import FeMoco.Boolean

/-! Signed-basis controller semantics. The model deliberately has no Givens,
reflection, arbitrary amplitudes or chemistry claim. Every outcome is a free
variable; conditional execution preserves disabled outcomes and reset obligations.
-/
namespace FeMoco.Controller
open Boolean

structure State (n : Nat) (α : Type) where
  wires : Fin n → α
  phase : α
  badReset : α

abbrev Symbolic (n : Nat) := State n Polynomial
abbrev Concrete (n : Nat) := State n Bool

def evaluate (v : Assignment) (s : Symbolic n) : Concrete n :=
  ⟨fun w => eval v (s.wires w), eval v s.phase, eval v s.badReset⟩

def put (s : State n α) (t : Fin n) (a : α) : State n α :=
  { s with wires := fun w => if w = t then a else s.wires w }

inductive Gate (n : Nat) where
  | x (t : Fin n)
  | cx (c t : Fin n)
  | ccx (a b t : Fin n)
  | z (t : Fin n)
  | cz (a b : Fin n)
  | ccz (a b c : Fin n)
  | neg
  | hmr (t : Fin n) (outcome : Nat)
  | reset (t : Fin n)
  | whenOutcome (outcome : Nat) (body : Gate n)
  deriving DecidableEq, Repr

def proper : Gate n → Prop
  | .cx c t => c ≠ t
  | .ccx a b t => a ≠ b ∧ a ≠ t ∧ b ≠ t
  | .cz a b => a ≠ b
  | .ccz a b c => a ≠ b ∧ a ≠ c ∧ b ≠ c
  | .whenOutcome _ g => proper g
  | _ => True
instance instProper (g : Gate n) : Decidable (proper g) :=
  match g with
  | .whenOutcome _ g => instProper g
  | .cx c t => inferInstanceAs (Decidable (c ≠ t))
  | .ccx a b t => inferInstanceAs (Decidable (a ≠ b ∧ a ≠ t ∧ b ≠ t))
  | .cz a b => inferInstanceAs (Decidable (a ≠ b))
  | .ccz a b c => inferInstanceAs (Decidable (a ≠ b ∧ a ≠ c ∧ b ≠ c))
  | .x _ | .z _ | .neg | .hmr .. | .reset _ => isTrue trivial

/-- Static upper bound; conditional gates are charged even when disabled. -/
def toffoli : Gate n → Nat
  | .ccx .. | .ccz .. => 1
  | .whenOutcome _ g => toffoli g
  | _ => 0

def measurements : Gate n → Nat
  | .hmr .. => 1
  | .whenOutcome _ g => measurements g
  | _ => 0

/-- Actual conditional counts in this fixed-record model. -/
def executedToffoli (v : Assignment) : Gate n → Nat
  | .whenOutcome k g => if v k then executedToffoli v g else 0
  | g => toffoli g

def executedMeasurements (v : Assignment) : Gate n → Nat
  | .whenOutcome k g => if v k then executedMeasurements v g else 0
  | g => measurements g

 theorem executedToffoli_le (v : Assignment) (g : Gate n) :
    executedToffoli v g ≤ toffoli g := by
  induction g with
  | whenOutcome k g ih =>
    simp only [executedToffoli, toffoli]
    split <;> omega
  | _ => exact Nat.le_refl _

 theorem executedMeasurements_le (v : Assignment) (g : Gate n) :
    executedMeasurements v g ≤ measurements g := by
  induction g with
  | whenOutcome k g ih =>
    simp only [executedMeasurements, measurements]
    split <;> omega
  | _ => exact Nat.le_refl _

 theorem programToffoli_le (v : Assignment) (p : List (Gate n)) :
    (p.map (executedToffoli v)).sum ≤ (p.map toffoli).sum := by
  induction p with
  | nil => exact Nat.le_refl _
  | cons g gs ih =>
    simp only [List.map_cons, List.sum_cons]
    exact Nat.add_le_add (executedToffoli_le v g) ih

 theorem programMeasurements_le (v : Assignment) (p : List (Gate n)) :
    (p.map (executedMeasurements v)).sum ≤ (p.map measurements).sum := by
  induction p with
  | nil => exact Nat.le_refl _
  | cons g gs ih =>
    simp only [List.map_cons, List.sum_cons]
    exact Nat.add_le_add (executedMeasurements_le v g) ih

def choose (c a b : Polynomial) : Polynomial := add b (mul c (add a b))
def disj (a b : Polynomial) : Polynomial := add (add a b) (mul a b)

 theorem eval_choose (v : Assignment) (c a b : Polynomial) :
    eval v (choose c a b) = (if eval v c then eval v a else eval v b) := by
  simp only [choose, eval_add, eval_mul]
  cases eval v c <;> cases eval v a <;> cases eval v b <;> rfl
 theorem eval_disj (v : Assignment) (a b : Polynomial) :
    eval v (disj a b) = (eval v a || eval v b) := by
  simp only [disj, eval_add, eval_mul]
  cases eval v a <;> cases eval v b <;> rfl

def merge (c : Polynomial) (a b : Symbolic n) : Symbolic n :=
  ⟨fun w => choose c (a.wires w) (b.wires w),
    choose c a.phase b.phase, choose c a.badReset b.badReset⟩

def symbolic (g : Gate n) (s : Symbolic n) : Symbolic n :=
  match g with
  | .x t => put s t (neg (s.wires t))
  | .cx c t => put s t (add (s.wires t) (s.wires c))
  | .ccx a b t => put s t (add (s.wires t) (mul (s.wires a) (s.wires b)))
  | .z t => { s with phase := add s.phase (s.wires t) }
  | .cz a b => { s with phase := add s.phase (mul (s.wires a) (s.wires b)) }
  | .ccz a b c => { s with phase := add s.phase (mul (mul (s.wires a) (s.wires b)) (s.wires c)) }
  | .neg => { s with phase := neg s.phase }
  | .hmr t k => put { s with phase := add s.phase (mul (atom k) (s.wires t)) } t []
  | .reset t => { s with badReset := disj s.badReset (s.wires t) }
  | .whenOutcome k g => merge (atom k) (symbolic g s) s

def concrete (v : Assignment) (g : Gate n) (s : Concrete n) : Concrete n :=
  match g with
  | .x t => put s t (!s.wires t)
  | .cx c t => put s t (s.wires t ^^ s.wires c)
  | .ccx a b t => put s t (s.wires t ^^ (s.wires a && s.wires b))
  | .z t => { s with phase := s.phase ^^ s.wires t }
  | .cz a b => { s with phase := s.phase ^^ (s.wires a && s.wires b) }
  | .ccz a b c => { s with phase := s.phase ^^ ((s.wires a && s.wires b) && s.wires c) }
  | .neg => { s with phase := !s.phase }
  | .hmr t k => put { s with phase := s.phase ^^ (v k && s.wires t) } t false
  | .reset t => { s with badReset := s.badReset || s.wires t }
  | .whenOutcome k g => if v k then concrete v g s else s

@[simp] theorem evaluate_put (v : Assignment) (s : Symbolic n) (t : Fin n) (p : Polynomial) :
    evaluate v (put s t p) = put (evaluate v s) t (eval v p) := by
  simp only [evaluate, put]
  congr 1
  funext w
  split <;> rfl

 theorem evaluate_merge (v : Assignment) (c : Polynomial) (a b : Symbolic n) :
    evaluate v (merge c a b) = (if eval v c then evaluate v a else evaluate v b) := by
  cases hc : eval v c <;> simp [evaluate, merge, eval_choose, hc]

/-- The central interpreter theorem, including nonlinear outcome products. -/
 theorem symbolic_sound (v : Assignment) (g : Gate n) (s : Symbolic n) :
    evaluate v (symbolic g s) = concrete v g (evaluate v s) := by
  induction g with
  | whenOutcome k g ih =>
    simp only [symbolic, concrete, evaluate_merge, eval_variable, ih]
  | _ =>
    simp only [symbolic, concrete, evaluate_put, eval_add, eval_mul,
      eval_neg, eval]
    simp [evaluate, eval_add, eval_mul, eval_neg, eval_disj, eval_variable]

def runSymbolic (p : List (Gate n)) (s : Symbolic n) : Symbolic n := p.foldl (fun s g => symbolic g s) s
def runConcrete (v : Assignment) (p : List (Gate n)) (s : Concrete n) : Concrete n :=
  p.foldl (fun s g => concrete v g s) s

 theorem run_sound (v : Assignment) (p : List (Gate n)) (s : Symbolic n) :
    evaluate v (runSymbolic p s) = runConcrete v p (evaluate v s) := by
  induction p generalizing s with
  | nil => rfl
  | cons g gs ih =>
    simp only [runSymbolic, runConcrete, List.foldl_cons]
    rw [show evaluate v (List.foldl (fun s g => symbolic g s) (symbolic g s) gs) =
      runConcrete v gs (evaluate v (symbolic g s)) from ih (symbolic g s)]
    rw [symbolic_sound]
    rfl

/-- Check every wire, the sign, and the accumulated dirty-reset obligation. -/
def check (a b : Symbolic n) : Bool :=
  (List.finRange n).all (fun w => checkEqual (a.wires w) (b.wires w)) &&
  checkEqual a.phase b.phase && checkEqual a.badReset []

 theorem check_sound (a b : Symbolic n) (h : check a b = true) (v : Assignment) :
    (evaluate v a).wires = (evaluate v b).wires ∧
    (evaluate v a).phase = (evaluate v b).phase ∧ (evaluate v a).badReset = false := by
  simp only [check, Bool.and_eq_true] at h
  refine ⟨?_, checkEqual_sound _ _ h.1.2 v, ?_⟩
  · funext w
    exact checkEqual_sound _ _ ((List.all_eq_true.mp h.1.1) w (List.mem_finRange w)) v
  · exact checkEqual_sound _ _ h.2 v

structure Bounds where
  toffoli : Nat
  measurements : Nat
  deriving Repr

/-- All resources and the proof concern precisely p. Static distinct-wire n is
kept separate from the challenge's sampled mean and peak-live conventions. -/
structure Certificate (p : List (Gate n)) (initial expected : Symbolic n) (bounds : Bounds) : Prop where
  physical : ∀ g ∈ p, proper g
  accepted : check (runSymbolic p initial) expected = true
  toffoli_le : (p.map toffoli).sum ≤ bounds.toffoli
  measurements_le : (p.map measurements).sum ≤ bounds.measurements

 theorem Certificate.correct {p : List (Gate n)} {initial expected : Symbolic n} {bounds}
    (h : Certificate p initial expected bounds) (v : Assignment) :
    (runConcrete v p (evaluate v initial)).wires = (evaluate v expected).wires ∧
    (runConcrete v p (evaluate v initial)).phase = (evaluate v expected).phase ∧
    (runConcrete v p (evaluate v initial)).badReset = false := by
  have hs := check_sound _ _ h.accepted v
  rwa [run_sound] at hs

 theorem Certificate.resources {p : List (Gate n)} {initial expected : Symbolic n} {bounds}
    (h : Certificate p initial expected bounds) (v : Assignment) :
    (p.map (executedToffoli v)).sum ≤ bounds.toffoli ∧
    (p.map (executedMeasurements v)).sum ≤ bounds.measurements :=
  ⟨Nat.le_trans (programToffoli_le v p) h.toffoli_le,
    Nat.le_trans (programMeasurements_le v p) h.measurements_le⟩

end FeMoco.Controller

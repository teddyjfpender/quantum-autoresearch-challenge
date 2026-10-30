import FeMoco.Controller

namespace FeMoco.Examples
open Boolean Controller

def initial : Symbolic 3 :=
  ⟨fun w => if w.val = 0 then atom 0 else if w.val = 1 then atom 1 else [], atom 3, []⟩

def andErase : List (Gate 3) :=
  [.ccx 0 1 2, .hmr 2 2, .whenOutcome 2 (.cz 0 1), .reset 2]

 theorem andErase_certificate : Certificate andErase initial initial ⟨1, 1⟩ where
  physical := by decide
  accepted := by decide
  toffoli_le := by decide
  measurements_le := by decide

 theorem andErase_correct (v : Assignment) :
    (runConcrete v andErase (evaluate v initial)).wires = (evaluate v initial).wires ∧
    (runConcrete v andErase (evaluate v initial)).phase = (evaluate v initial).phase ∧
    (runConcrete v andErase (evaluate v initial)).badReset = false :=
  andErase_certificate.correct v

/-- Erasure alone restores bits, but leaves an outcome-dependent sign. -/
 theorem missing_correction_rejected :
    check (runSymbolic [.ccx 0 1 2, .hmr 2 2, .reset 2] initial) initial = false := by decide

/-- A reset is an obligation, never an unchecked way to erase a dirty wire. -/
 theorem dirty_reset_rejected :
    check (runSymbolic [.ccx 0 1 2, .reset 2] initial) initial = false := by decide

 theorem wrong_correction_rejected :
    check (runSymbolic [.ccx 0 1 2, .hmr 2 2, .whenOutcome 2 (.z 0), .reset 2] initial)
      initial = false := by decide

 theorem duplicate_controls_rejected : ¬ proper (.ccx (0 : Fin 3) 0 2) := by decide

/-- Nonlinear measurement products are retained, rather than assumed linear. -/
 theorem nonlinear_outcome_defect_rejected :
    checkEqual (mul (atom 32) (mul (atom 33) (atom 34))) [] = false := by decide

def thirtyTwoOutcomes : Polynomial :=
  (List.range 32).foldl (fun p k => mul p (atom (k+32))) one

 theorem thirty_two_outcome_defect_rejected : checkEqual thirtyTwoOutcomes [] = false := by decide
 theorem thirty_two_outcome_witness : eval (fun _ => true) thirtyTwoOutcomes = true := by decide

 theorem cancellation_checked : checkEqual (add (atom 0) (atom 0)) [] = true := by decide
 theorem idempotence_checked : checkEqual (mul (atom 0) (atom 0)) (atom 0) = true := by decide

end FeMoco.Examples

import Std

/-! A small exact Boolean-polynomial kernel. Search and canonicalisation are not
trusted: the soundness theorems below justify the operations on every assignment.
Variables include arbitrary controller inputs and independent measurement outcomes.
-/
namespace FeMoco.Boolean

abbrev Assignment := Nat → Bool
abbrev Monomial := List Nat
abbrev Polynomial := List Monomial

def evalMonomial (v : Assignment) : Monomial → Bool
  | [] => true
  | x :: xs => v x && evalMonomial v xs

def eval (v : Assignment) : Polynomial → Bool
  | [] => false
  | m :: ms => evalMonomial v m ^^ eval v ms

/-- Insert a variable, using x*x=x. -/
def insertVar (x : Nat) : Monomial → Monomial
  | [] => [x]
  | y :: ys => if x = y then y :: ys
    else if x < y then x :: y :: ys else y :: insertVar x ys

/-- Insert a coefficient, using a+a=0. The order is only an optimisation. -/
def toggle (m : Monomial) : Polynomial → Polynomial
  | [] => [m]
  | n :: ns => if m = n then ns
    else if compare m n == .lt then m :: n :: ns else n :: toggle m ns

def add (p q : Polynomial) : Polynomial := p.foldr toggle q

def productMonomial (m n : Monomial) : Monomial := m.foldr insertVar n

def scale (m : Monomial) (p : Polynomial) : Polynomial :=
  p.foldr (fun n acc => toggle (productMonomial m n) acc) []

def mul (p q : Polynomial) : Polynomial := p.foldr (fun m acc => add (scale m q) acc) []

def one : Polynomial := [[]]
def atom (x : Nat) : Polynomial := [[x]]
def neg (p : Polynomial) : Polynomial := add one p

 theorem eval_insertVar (v : Assignment) (x : Nat) (m : Monomial) :
    evalMonomial v (insertVar x m) = (v x && evalMonomial v m) := by
  induction m with
  | nil => simp [insertVar, evalMonomial]
  | cons y ys ih =>
    by_cases h : x = y
    · subst y; simp [insertVar, evalMonomial]
    · simp only [insertVar, h, ↓reduceIte]
      split
      · rfl
      · rw [evalMonomial, ih, evalMonomial]
        cases v x <;> cases v y <;> rfl

 theorem eval_toggle (v : Assignment) (m : Monomial) (p : Polynomial) :
    eval v (toggle m p) = (evalMonomial v m ^^ eval v p) := by
  induction p with
  | nil => simp [toggle, eval]
  | cons n ns ih =>
    by_cases h : m = n
    · subst n; simp [toggle, eval]
    · simp only [toggle, h, ↓reduceIte]
      split
      · rfl
      · rw [eval, ih, eval]
        cases evalMonomial v m <;> cases evalMonomial v n <;> cases eval v ns <;> rfl

 theorem eval_add (v : Assignment) (p q : Polynomial) :
    eval v (add p q) = (eval v p ^^ eval v q) := by
  induction p with
  | nil => simp [add, eval]
  | cons m ms ih =>
    simp only [add, List.foldr_cons, eval_toggle, eval]
    rw [show eval v (List.foldr toggle q ms) = (eval v ms ^^ eval v q) from ih]
    exact (Bool.xor_assoc _ _ _).symm

 theorem eval_productMonomial (v : Assignment) (m n : Monomial) :
    evalMonomial v (productMonomial m n) = (evalMonomial v m && evalMonomial v n) := by
  induction m with
  | nil => simp [productMonomial, evalMonomial]
  | cons x xs ih =>
    simp only [productMonomial, List.foldr_cons, eval_insertVar]
    rw [show evalMonomial v (List.foldr insertVar n xs) =
      (evalMonomial v xs && evalMonomial v n) from ih]
    simp [evalMonomial, Bool.and_assoc]

 theorem eval_scale (v : Assignment) (m : Monomial) (p : Polynomial) :
    eval v (scale m p) = (evalMonomial v m && eval v p) := by
  induction p with
  | nil => simp [scale, eval]
  | cons n ns ih =>
    simp only [scale, List.foldr_cons, eval_toggle, eval_productMonomial]
    rw [show eval v (List.foldr _ [] ns) = (evalMonomial v m && eval v ns) from ih]
    simp only [eval]
    cases evalMonomial v m <;> cases evalMonomial v n <;> cases eval v ns <;> rfl

 theorem eval_mul (v : Assignment) (p q : Polynomial) :
    eval v (mul p q) = (eval v p && eval v q) := by
  induction p with
  | nil => simp [mul, eval]
  | cons m ms ih =>
    simp only [mul, List.foldr_cons, eval_add, eval_scale]
    rw [show eval v (List.foldr _ [] ms) = (eval v ms && eval v q) from ih]
    simp only [eval]
    cases evalMonomial v m <;> cases eval v ms <;> cases eval v q <;> rfl

@[simp] theorem eval_one (v : Assignment) : eval v one = true := rfl
@[simp] theorem eval_variable (v : Assignment) (x : Nat) : eval v (atom x) = v x := by
  simp [atom, eval, evalMonomial]
 theorem eval_neg (v : Assignment) (p : Polynomial) : eval v (neg p) = !eval v p := by
  rw [neg, eval_add, eval_one]; cases eval v p <;> rfl

/-- An executable acceptance check, justified for ALL assignments. No completeness
claim is required: a noncanonical but valid expression may be refused. -/
def checkEqual (p q : Polynomial) : Bool := (add p q).isEmpty

 theorem checkEqual_sound (p q : Polynomial) (h : checkEqual p q = true) :
    ∀ v, eval v p = eval v q := by
  intro v
  have hz : add p q = [] := List.isEmpty_iff.mp h
  have he := eval_add v p q
  rw [hz] at he
  simp only [eval] at he
  cases hp : eval v p <;> cases hq : eval v q <;> simp_all

end FeMoco.Boolean

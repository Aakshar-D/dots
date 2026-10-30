# Projects

Projects, tasks and to-dos, shared by the person and their dots (app library spec section 8).
It comes with Dots, which updates it with each release until the person accepts a dot's change to
it; from then on it is the person's.

- `migrations/`: the data. A change keeps these files byte for byte and adds new ones after them.
- `logic.js`: the tools. Every change to a task or a subtask writes an Activity row with the
  values before and after; a dot cannot mark a task Done.
- `tests/`: run with every proposed change.

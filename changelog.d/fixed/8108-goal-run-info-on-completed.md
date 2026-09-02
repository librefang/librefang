Show a goal's autonomous run outcome after the goal completes, instead of hiding it exactly when it is worth reading.
The inline run state was gated on `status !== "completed"`, but `GoalRunPhase::Finished` is documented as "the goal reached `Completed`/`Cancelled`" — so the `finished` badge and its five translations were unreachable, and an operator could never see whether a run ended on its own or stopped at the iteration cap.
The component already renders nothing for a goal with no run, so a completed goal that never ran stays as quiet as it was. (#8108) (@houko)

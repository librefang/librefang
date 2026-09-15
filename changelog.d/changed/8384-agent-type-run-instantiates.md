Creating an agent from an agent type now opens the create flow with that type already chosen, instead of asking which existing agent to fork.
The control sits on a type, yet it opened a modal whose first field was a dropdown of the agents that already exist, so the operator answered a question they had not asked and then re-picked the same type from a different dropdown on a different page.
It now opens the Agents page's create drawer on the template tab with the type preselected, carried in a `template` search param that is cleared when the drawer closes so a second press re-opens it.
The ephemeral-run modal goes with it, because that control was its only entry point (#8384) (@DaBlitzStein)

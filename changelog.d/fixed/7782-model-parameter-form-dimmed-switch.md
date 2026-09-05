The model parameter form no longer fades the switch that activates each row, so a model with no overrides stops looking like a screen that failed to load.
Every row starts inactive until the model carries an override for that field, and the inactive styling sat on the row container — which also holds the `role="switch"` button, drawn in the divider token and then faded to 40% on top of that.
The only affordance that could undo the fade was the least visible thing on the row, and with all seven rows inactive by default that was the form's opening state.
The dimming now applies to the label, the number input, the slider and the ticks, the switch keeps full opacity in both states, and its off state uses a token that reads as a control rather than as a hairline (#7782) (@DaBlitzStein)

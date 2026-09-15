`GET /api/agents` and `GET /api/agents/{id}` now carry `provisioned`, naming the deployment file that declares the agent, or `null` when it is the operator's own.
Eleven manifest-writing routes answer `423 Locked` on a provisioned agent and the kernel has always known which agents those are, but the payload never said, so a client could not tell before trying: an operator would type an emoji and save, or pick an image and upload the whole thing, only to be refused at the end by something that was never going to work.
`source` is the declaring file, which is the one thing a surface needs beyond "you cannot" — it says where to go and change it instead.
The field is present and `null` rather than absent when provisioning is switched off, so a client never has to distinguish two spellings of the same answer.
The ten of those eleven routes whose OpenAPI said nothing about the refusal now document the `423`, so a generated client stops treating it as an unmodelled failure (#8379) (@houko)

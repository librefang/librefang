Tie Fly deployment progress to the real request lifecycle instead of a cosmetic timer. (@houko)
The deploy page previously marked one setup step complete every 1.5 seconds even while `/api/deploy` was still pending, and left that interval running when the form unmounted.
Pending deployments now show only the request as active, mark completion only after a successful response, and abort plus ignore late results when the form unmounts.

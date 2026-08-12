Add default network timeouts to the generated Rust SDK.
The default reqwest client previously had no connect timeout, and ordinary API requests could wait indefinitely for a server that accepted a connection but never responded.
All requests now use a 10-second connect timeout, while non-streaming calls additionally use a 60-second total timeout; SSE bodies remain exempt from the total deadline so long-lived streams continue normally (#6836) (@houko)

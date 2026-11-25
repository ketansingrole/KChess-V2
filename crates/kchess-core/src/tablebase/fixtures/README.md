# Syzygy fixtures of the tablebase tests

Four tables of three pieces (`KQvK`, `KRvK`, each its win/draw/loss `.rtbw` and distance-to-zero
`.rtbz` file), as Lichess's mirror publishes them at
`https://tablebase.lichess.ovh/tables/standard/3-4-5-{wdl,dtz}/`. Each matches its line in
`../files.txt`. The tests probe them (`tablebase/tests.rs`) and serve them as the mirror would
(the core's `tablebase` test).

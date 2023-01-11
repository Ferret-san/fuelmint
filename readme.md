# fuelmint
**fuelmint** is a scalable and interoperable Fuel library, built using Cosmos SDK and Rollmint and created with [Ignite CLI](https://ignite.com/cli).

# How it works

fuelmint works by creating an ABCI wrapper for the FuelVM, which then connects to Rollmint thorugh a shim implemented as a cosmos-sdk module, which is responsible for relaying the ABCI requests to the FuelVM wrapper.

# Test it

You will first need to run a local Celestia container, you can find one here [here](https://github.com/celestiaorg/go-cnc).

Once you have the `go-cnc` repo, run `docker compose -f ./integration_test/docker/test-docker-compose.yml up` at the root level of the `go-cnc` directory.

## Steps to test fuelmint

Once you have the local Celestia container running, you will need two terminal sessions. You will want to first run `cargo run --bin app` at the root of `fuelmint` in one session, and then run the following commands in the other sessions:

- rm -rf /tmp/fuelmint/
- TMHOME="/tmp/fuelmint" tendermint init
- NAMESPACE_ID=$(echo $RANDOM | md5sum | head -c 16; echo;)
- ./node -config "/tmp/fuelmint/config/config.toml" -namespace_id $NAMESPACE_ID -da_start_height 1

If everything is running, you can try sending this transaction to fuelmint: 
```
curl -s 'localhost:26657/broadcast_tx_commit?tx=0000000000000000000000000000000000000000000f42400000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000504000ca504400ba3341100024040000'
```

You should then be able to get a response from curl, and also see the DeliverTx request and response in the terminal where you ran `cargo run --bin app`


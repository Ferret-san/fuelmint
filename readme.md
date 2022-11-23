# fuelmint
**fuelmint** is a scalable and interoperable Fuel library, built using Cosmos SDK and Rollmint and created with [Ignite CLI](https://ignite.com/cli).

# How it works

fuelmint works by creating an ABCI wrapper for the FuelVM, which then connects to Rollmint thorugh a shim implemented as a cosmos-sdk module, which is responsible for relaying the ABCI requests to the FuelVM wrapper.

# run tests

To run the basic test, open your cli and the following at the root level of the repo:

`cargo test -- --nocapture` 

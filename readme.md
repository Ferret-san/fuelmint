# Fuelmint

**Fuelmint** is a tool to run FuelVM sovereign rollups on top of Celestia by leveraging Rollkit that allows you to use Fuel's toolchain to develop, test, deploy, and interact with Sway smart contracts.

# How it works

![Fuelmint-diagram](https://user-images.githubusercontent.com/31937514/214973752-e8e5283e-706e-4da9-867d-f6298e2b883c.png)

Fuelmint works thanks to the following components:
- FuelVM ABCI wrapper using `tower-abci`
- A set of services (graph_ql client, txpool) that allow it Fuel tools such as `forc` to communicate with the rollup
- A socket client as an ABCI app written in Go, which allows Rollkit to communicate with our `tower-abci` server

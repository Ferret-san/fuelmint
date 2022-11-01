package keeper_test

import (
	"context"
	"testing"

	keepertest "fuelmint/testutil/keeper"
	"fuelmint/x/fuelmint/keeper"
	"fuelmint/x/fuelmint/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
)

func setupMsgServer(t testing.TB) (types.MsgServer, context.Context) {
	k, ctx := keepertest.FuelmintKeeper(t)
	return keeper.NewMsgServerImpl(*k), sdk.WrapSDKContext(ctx)
}

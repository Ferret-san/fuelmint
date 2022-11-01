package keeper_test

import (
	"testing"

	testkeeper "fuelmint/testutil/keeper"
	"fuelmint/x/fuelmint/types"
	"github.com/stretchr/testify/require"
)

func TestGetParams(t *testing.T) {
	k, ctx := testkeeper.FuelmintKeeper(t)
	params := types.DefaultParams()

	k.SetParams(ctx, params)

	require.EqualValues(t, params, k.GetParams(ctx))
}

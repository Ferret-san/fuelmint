package fuelmint_test

import (
	"testing"

	keepertest "fuelmint/testutil/keeper"
	"fuelmint/testutil/nullify"
	"fuelmint/x/fuelmint"
	"fuelmint/x/fuelmint/types"
	"github.com/stretchr/testify/require"
)

func TestGenesis(t *testing.T) {
	genesisState := types.GenesisState{
		Params: types.DefaultParams(),

		// this line is used by starport scaffolding # genesis/test/state
	}

	k, ctx := keepertest.FuelmintKeeper(t)
	fuelmint.InitGenesis(ctx, *k, genesisState)
	got := fuelmint.ExportGenesis(ctx, *k)
	require.NotNil(t, got)

	nullify.Fill(&genesisState)
	nullify.Fill(got)

	// this line is used by starport scaffolding # genesis/test/assert
}

// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::cli_state::CliState;
use crate::view::BlockView;
use crate::StarcoinOpt;
use anyhow::Result;
use scmd::{CommandAction, ExecContext};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "list_block")]
pub struct GetOpt {
    #[structopt(name = "number", long, default_value = "0")]
    number: usize,
    #[structopt(name = "count", long, default_value = "1")]
    count: usize,
}

pub struct ListBlockCommand;

impl CommandAction for ListBlockCommand {
    type State = CliState;
    type GlobalOpt = StarcoinOpt;
    type Opt = GetOpt;
    type ReturnItem = Vec<BlockView>;

    fn run(
        &self,
        ctx: &ExecContext<Self::State, Self::GlobalOpt, Self::Opt>,
    ) -> Result<Self::ReturnItem> {
        let client = ctx.state().client();
        let opt = ctx.opt();
        let blocks = client.chain_get_blocks_by_number(opt.number as u64, opt.count as u64)?;
        let blockview = blocks
            .iter()
            .map(|block| BlockView::from(block.clone()))
            .collect();
        Ok(blockview)
    }
}

// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::cli_state::CliState;
use crate::StarcoinOpt;
use anyhow::Result;
use scmd::{CommandAction, ExecContext};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "sign_txn")]
pub struct SignTxnOpt {}

pub struct SignTxnCommand;

impl CommandAction for SignTxnCommand {
    type State = CliState;
    type GlobalOpt = StarcoinOpt;
    type Opt = SignTxnOpt;
    type ReturnItem = ();

    fn run(&self, ctx: &ExecContext<Self::State, Self::GlobalOpt, Self::Opt>) -> Result<()> {
        //let client = ctx.state().client();
        let _opt = ctx.opt();
        //let account = client.account_create(ctx.opt().password.clone())?;
        println!("TODO sign txn command");
        Ok(())
    }
}

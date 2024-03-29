// LNP/BP universal invoice library implementing LNPBP-38 standard
// Written in 2021 by
//     Dr. Maxim Orlovsky <orlovsky@pandoracore.com>
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the MIT License
// along with this software.
// If not, see <https://opensource.org/licenses/MIT>.

#[macro_use]
extern crate clap;
extern crate serde_crate as serde;

use clap::Parser;
use serde::Serialize;
use std::fmt::{self, Debug, Display, Formatter};
use std::io::{self, Read};
use std::str::FromStr;

use base58::{FromBase58, ToBase58};
use bitcoin::hashes::hex::{self, FromHex, ToHex};
use bitcoin::OutPoint;
use bp::seals::txout::blind::RevealedSeal;
use invoice::{Beneficiary, Invoice};
use lnpbp::chain::AssetId;
use strict_encoding::{StrictDecode, StrictEncode};

#[derive(Parser, Clone, Debug)]
#[clap(
    name = "invoice",
    bin_name = "invoice",
    author,
    version,
    about = "Command-line tool for working with LNP/BP invoicing"
)]
pub struct Opts {
    /// Command to execute
    #[clap(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Clone, PartialEq, Eq, Debug)]
pub enum Command {
    /// Creates new invoice
    Create {
        /// Beneficiary data
        beneficiary: Beneficiary,

        /// Amount, in satoshis or smallest asset division
        amount: Option<u64>,

        /// Asset, if not Bitcoin
        asset: Option<rgb::ContractId>,
    },

    /// Converting between different representations of invoice data
    Convert {
        /// Invoice data; if none are given reads from STDIN
        invoice: Option<String>,

        /// Formatting of the input invoice data
        #[clap(short, long, default_value = "bech32")]
        input: Format,

        /// Formatting for the output invoice data
        #[clap(short, long, default_value = "yaml")]
        output: Format,
    },

    /// Converts RGB asset id between representations
    RgbConvert {
        /// Asset id in any format
        asset: Option<String>,

        /// Formatting of the input invoice data
        #[clap(short, long, default_value = "hex")]
        input: Format,

        /// Formatting for the output invoice data
        #[clap(short, long, default_value = "bech32")]
        output: Format,
    },

    /// Creates blinded UTXO representation from a given outpoint
    Conceal {
        /// UTXO to conceal
        outpoint: OutPoint,
    },
}

/// Formatting of the data
#[derive(ArgEnum, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Debug)]
pub enum Format {
    /// Format according to the rust debug rules
    Debug,

    /// Format using Bech32 representation
    Bech32m,

    /// Format using Base58 encoding
    Base58,

    /// Format using Base64 encoding
    Base64,

    /// Format as YAML
    Yaml,

    /// Format as JSON
    Json,

    /// Format according to the strict encoding rules
    Hexadecimal,

    /// Format as a rust array (using hexadecimal byte values)
    Rust,

    /// Produce binary (raw) output according to LNPBP-39 serialization rules
    Raw,
}

impl Display for Format {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Format::Debug => f.write_str("debug"),
            Format::Base58 => f.write_str("base58"),
            Format::Base64 => f.write_str("base64"),
            Format::Bech32m => f.write_str("bech32"),
            Format::Yaml => f.write_str("yaml"),
            Format::Json => f.write_str("json"),
            Format::Hexadecimal => f.write_str("hex"),
            Format::Rust => f.write_str("rust"),
            Format::Raw => f.write_str("raw"),
        }
    }
}

impl FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.trim().to_lowercase().as_str() {
            "debug" => Format::Debug,
            "base58" => Format::Base58,
            "base64" => Format::Base64,
            "bech32" => Format::Bech32m,
            "yaml" => Format::Yaml,
            "json" => Format::Json,
            "hex" => Format::Hexadecimal,
            "raw" | "bin" => Format::Raw,
            "rust" => Format::Rust,
            other => Err(format!("Unknown format: {}", other))?,
        })
    }
}

fn input_read<T>(data: Option<String>, format: Format) -> Result<T, String>
where
    T: FromStr + StrictDecode + for<'de> serde::Deserialize<'de>,
    <T as FromStr>::Err: Display,
{
    let data = data
        .map(|d| d.as_bytes().to_vec())
        .ok_or(String::default())
        .or_else(|_| -> Result<Vec<u8>, String> {
            let mut buf = Vec::new();
            io::stdin()
                .read_to_end(&mut buf)
                .as_ref()
                .map_err(io::Error::to_string)?;
            Ok(buf)
        })?;
    let s = &String::from_utf8_lossy(&data);
    Ok(match format {
        Format::Bech32m => T::from_str(s).map_err(|err| err.to_string())?,
        Format::Base58 => {
            T::strict_deserialize(s.from_base58().map_err(|err| {
                format!("Incorrect Base58 encoding: {:?}", err)
            })?)
            .map_err(|err| format!("Wrong invoice data: {}", err))?
        }
        Format::Base64 => T::strict_deserialize(
            &base64::decode(&data)
                .map_err(|err| format!("Incorrect Base64 encoding: {}", err))?,
        )
        .map_err(|err| format!("Wrong invoice data: {}", err))?,
        Format::Yaml => {
            serde_yaml::from_str(s).map_err(|err| err.to_string())?
        }
        Format::Json => {
            serde_json::from_str(s).map_err(|err| err.to_string())?
        }
        Format::Hexadecimal => T::strict_deserialize(
            Vec::<u8>::from_hex(s)
                .as_ref()
                .map_err(hex::Error::to_string)?,
        )
        .map_err(|err| format!("Wrong invoice data: {}", err))?,
        Format::Raw => T::strict_deserialize(&data)
            .map_err(|err| format!("Wrong invoice data: {}", err))?,
        _ => Err(format!("Can't read data from {} format", format))?,
    })
}

fn output_write<T>(
    mut f: impl io::Write,
    data: T,
    format: Format,
) -> Result<(), String>
where
    T: Debug + Display + Serialize + StrictEncode,
{
    let strict = data.strict_serialize().map_err(|err| err.to_string())?;
    match format {
        Format::Debug => writeln!(f, "{:#?}", data),
        Format::Bech32m => writeln!(f, "{}", data),
        Format::Base58 => writeln!(f, "{}", strict.to_base58()),
        Format::Base64 => writeln!(f, "{}", base64::encode(&strict)),
        Format::Yaml => writeln!(
            f,
            "{}",
            serde_yaml::to_string(&data)
                .as_ref()
                .map_err(serde_yaml::Error::to_string)?
        ),
        Format::Json => writeln!(
            f,
            "{}",
            serde_json::to_string(&data)
                .as_ref()
                .map_err(serde_json::Error::to_string)?
        ),
        Format::Hexadecimal => writeln!(f, "{}", strict.to_hex()),
        Format::Rust => writeln!(f, "{:#04X?}", strict),
        Format::Raw => data
            .strict_encode(f)
            .map(|_| ())
            .map_err(|_| io::Error::from_raw_os_error(0)),
    }
    .as_ref()
    .map_err(io::Error::to_string)?;
    Ok(())
}

fn main() -> Result<(), String> {
    let opts = Opts::parse();

    match opts.command {
        Command::Create {
            beneficiary,
            amount,
            asset,
        } => {
            let invoice =
                Invoice::new(beneficiary, amount, asset.map(AssetId::from));
            output_write(io::stdout(), invoice, Format::Bech32m)?;
        }
        Command::Convert {
            invoice,
            input,
            output,
        } => {
            let invoice: Invoice = input_read(invoice, input)?;
            output_write(io::stdout(), invoice, output)?;
        }
        Command::RgbConvert {
            asset,
            input,
            output,
        } => {
            let asset: rgb::ContractId = input_read(asset, input)?;
            output_write(io::stdout(), asset, output)?;
        }
        Command::Conceal { outpoint } => {
            let seal = RevealedSeal::from(outpoint);
            println!("{}", seal);
            println!("{}", seal.to_concealed_seal());
        }
    }

    Ok(())
}

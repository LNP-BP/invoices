use amplify::Wrapper;
use bitcoin::{
    hashes::{sha256, Hash},
    secp256k1::rand::{self},
};
use lightning::ln::PaymentSecret;
use lightning_invoice::{Currency, InvoiceBuilder, RawInvoice};
use lnpbp::chain::Chain;
use std::convert::TryFrom;

use crate::{Beneficiary, Invoice};

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Display, Error)]
#[display(doc_comments)]
pub enum InvoiceError {
    /// Invoice beneficiary is not compatible
    UnknownBeneficiary,
    /// Invoice chain is not compatible
    UnknownChain,
    /// Invoice cannot contains payment_hash field
    MissingPaymentHash,
    /// Raw invoice with missing parts
    ParserError,
}

impl TryFrom<Invoice> for RawInvoice {
    type Error = InvoiceError;

    fn try_from(invoice: Invoice) -> Result<Self, Self::Error> {
        if let Beneficiary::Bolt(params) = invoice.beneficiary() {
            let payment_hash = sha256::Hash::from_slice(&params.lock[..])
                .map_err(|_| InvoiceError::MissingPaymentHash);

            let min_final_cltv_expiry =
                params.min_final_cltv_expiry.unwrap_or_default();

            let currency = match params.network {
                Chain::Mainnet => Ok(Currency::Bitcoin),
                Chain::Testnet3 => Ok(Currency::BitcoinTestnet),
                Chain::Regtest(_) => Ok(Currency::Regtest),
                Chain::Signet => Ok(Currency::Signet),
                _ => Err(InvoiceError::UnknownChain),
            };

            let description = match invoice.purpose() {
                Some(desc) => desc,
                _ => "",
            };

            let payment_secret = match params.secret {
                Some(secret) => secret.as_inner().to_owned(),
                _ => rand::random(),
            };
            let payment_secret = PaymentSecret(payment_secret);
            let bolt11 = InvoiceBuilder::new(currency?)
                .description(description.to_owned())
                .amount_milli_satoshis(
                    invoice.amount().atomic_value().unwrap_or_default(),
                )
                .payment_hash(payment_hash?)
                .payment_secret(payment_secret)
                .current_timestamp()
                .min_final_cltv_expiry(min_final_cltv_expiry.into());

            bolt11.build_raw().map_err(|_| InvoiceError::ParserError)
        } else {
            Err(InvoiceError::UnknownBeneficiary)
        }
    }
}

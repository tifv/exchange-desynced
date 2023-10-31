//! A specialized imitation of `serde::ser`.

use std::io::Read;

use crate::{
    Exchange, ExchangeKind,
    table::{TableItem, TableSize},
};

use self::reader::Reader;

mod reader;
mod value;
mod decompress;

pub trait Error : std::error::Error + for<'s> From<&'s str> {}

pub mod error {
    use thiserror::Error;

    #[derive(Debug, Error)]
    #[error("Load error: {reason}")]
    pub struct Error {
        reason: String,
    }

    impl From<&str> for Error {
        fn from(reason: &str) -> Self {
            Self{reason: String::from(reason)}
        }
    }

    impl From<String> for Error {
        fn from(reason: String) -> Self {
            Self{reason}
        }
    }

    macro_rules! error_from_error {
        ($type:ty) => {
            impl From<$type> for Error {
                fn from(value: $type) -> Self {
                    Self::from(value.to_string())
                }
            }
        };
    }

    error_from_error!(crate::ascii::AsciiError);
    error_from_error!(crate::intlim::IntLimError);
    error_from_error!(std::str::Utf8Error);
    error_from_error!(std::io::Error);

    impl super::Error for Error {}

}

pub trait LoadKey : Sized {
    fn load_key<L: Loader>(loader: L) -> Result<Option<Self>, L::Error>;
}

pub trait Load : Sized {
    fn load<L: Loader>(loader: L) -> Result<Self, L::Error>;
    fn is_nil(&self) -> bool;
}

pub trait LoadTableIterator : TableSize + Iterator<
    Item=Result<Option<TableItem<Self::Key, Self::Value>>, Self::Error> >
{
    type Key: LoadKey;
    type Value: Load;
    type Error: Error;
}

pub trait KeyBuilder : Sized {
    type Value: LoadKey;
    fn build_integer<E: Error>(self, value: i32) -> Result<Self::Value, E>;
    fn build_string<E: Error>(self, value: &str) -> Result<Self::Value, E>;
}

pub trait Builder : Sized {
    type Key: LoadKey;
    type Value: Load;
    fn build_nil<E: Error>(self) -> Result<Self::Value, E>;
    fn build_boolean<E: Error>(self, value: bool) -> Result<Self::Value, E>;
    fn build_integer<E: Error>(self, value: i32) -> Result<Self::Value, E>;
    fn build_float<E: Error>(self, value: f64) -> Result<Self::Value, E>;
    fn build_string<E: Error>(self, value: &str) -> Result<Self::Value, E>;
    fn build_table<T, E: Error>(self, items: T) -> Result<Self::Value, E>
    where T: LoadTableIterator<Key=Self::Key, Value=Self::Value, Error=E>;
}

pub trait Loader {
    type Error: Error;
    fn load_value<B: Builder>( self,
        builder: B,
    ) -> Result<B::Value, Self::Error>;
    fn load_key<B: KeyBuilder>( self,
        builder: B,
    ) -> Result<Option<B::Value>, Self::Error>;
}

pub fn load_blueprint<P, B>(data: &str) -> Result<Exchange<P, B>, error::Error>
where P: Load, B: Load,
{
    let (kind, encoded_body) = decompress::decompress(data)?;
    Ok(match kind {
        ExchangeKind::Blueprint(()) =>
            Exchange::Blueprint(P::load(
                &mut value::Loader::new(Reader::from_slice(&encoded_body))
            )?),
        ExchangeKind::Behavior(()) =>
            Exchange::Behavior(B::load(
                &mut value::Loader::new(Reader::from_slice(&encoded_body))
            )?),
    })
}

#[cfg(test)]
mod test {
    use crate::value::Value;

    use super::super::dump::dump_blueprint;
    use super::load_blueprint;

    const DATA_1: &str = "\
        DSC22y1Z49l21IhQFh0oJ9l64TPfet44myv4377DXE0xACL43XfsVo13Q2e52uEK\
        v80XNctN4RLH2q3jfPpS2AEMU31gVJcw0JF1R03moTTo2DIJVW4VdGXN4DfvLt2J\
        Ji4x4LJQ2g2FglIy0adSA01jc2zu0VW7C52BuTh54RIo2s4dRP9027hoCf2g8gTR\
        4PDRnB2UeSwR26Sc3g4OsXKO3Sr04Y2hwMdg3AM1Sp0p2PHD2fo2tS3MDgqb3dpy\
        Le1gEH3y1ylKwg0HIFq91T8ONE0VcdXW3aIloJ2AH5324B5lWI25PEEV1aH4iP2k\
        NlBr3JSx3J0gFGx403B8xo2NDi0V25KKwQ0fj0xL39fMwO0fbCA01PKbYP3Cu57P\
        3pfZvK1x0M6z0xM1t90XCfBZ3FkvAH4GcVxw1RFYsn4eZAyj2idbiS3ps71P1gPs\
        Vd0CkS3Z23XL7T4MdoqZ2ymqOz0fdGIx2Q0rcR38K7pC10KdXu2TJ5f33gWjlj1y\
        pMDd3QlzdM3YdoW11U1hoB2l2U7T2P2T8W4ctY0a0Pcqe60WVSV31BowIl0h46Zd\
        1ME5sj2EppSX3toTlN2Rmtdi4XVV6O4arVHS3ILZia1oMpXw0tpPnE1VZuLe0IGC\
        112CCAVe3NIyZc1tABRc1YzBmu2Wt76c41Dsrq15A0kF0F1qC34Zjwdx0Ul3Og0i\
        vM2Z1nOXbO352YXD0roDDA2hTmk83tzqyF43w76T1Art1M4CE7qL0RnpOJ0e45E6\
        2YrOfd2hEeb510mhbc4TSYua41sEVu2eEQ9C1nLKHf475iAV3SFX153ENfFH1kfA\
        GJ1F1hd21laEpw4SCS8v2lHys03u1EYv1mK1f62z9Z3q20npE92OSB2v0oFLuj1c\
        96Nt1h0vTK0t1Tu62t4z7v0rTQ7C3UTyEN3Vicqb1j5msz0mjxqe2SaKQD2MavcV\
        2XBkFp2ScU1o4SiGUy0CZcjB1xVbdw0AfZzb0RetOD1xy49p354hT743hvqM4c4i\
        1Y3BBXhh0WEJxw27QirN32riX70giDyM21fYvC1jBtyT4KXout2F0sVD1beemU23\
        vycT1gw9ng4770z042l8pe2uLzoa2B4bKn2SHcSi3RU27V1kRten2lCrYF3o8Saz\
        242QpN0EkQ8a2r7HS03mjw9k3tESSx22g0600iHhKx1E0j9A4JfXld1GcaOJ2UiR\
        l740la5g0cx9mn0oe0eK3o8Vbj39qK2k0oun7F29ii4v275I3a02Pa9T04gPAZ\
    ";

    const DATA_2: &str = "\
        DSC2Az1Z49l210ZIJZ1CGTxo2wnGzt1BSpuq4TxlWR4ACY2C08sw230BCpOy0JSk\
        Xe2gAv5m0dPrZr4023vV3g84wB1L8ajW0Tm0wl1Npiyh3maDqZ3hYOFm2LcKI64D\
        bHHx1Hw01z4BzGgB2NmB0b2sLX0h10C6di3zg3UR4VqG9i2PlkPg12He480BSkI5\
        473GsK0ph4iu1gCxcA4VgUUG2UNttG2iaR4B0lXdY81gkBA81zx3te1MO0Yp4Iov\
        iA1vdCnx0PaTme4XPVBL2ExbH70Dy5lI2k2btG3gG6jl0ZCPOn2aSJy40hd3ui1Z\
        kfng4cjk7l3WR7xE40HAYV0XYT991WSqH10pKi432BwEHB3kVXQM2xnzOU3LGHwn\
        3HSYeN1A6ony3SHHA94D4MzS3drZRP1DxXb23JJyY32xyLoH2DFCMp4YWOJ83uNd\
        sh1jejhM42BBgd0jDxxo4YeG923ZqzJc0sDaoo23dwtd4eL5tF0NS6ZQ3k4Hvq0b\
        Uf2W3oUTvl3RM7132JOEBF0dKVt73VVQ8d3Us6rS4T87RG2IqDAm0Xmvaz4DP5Fk\
        0aOxxS0PtqRt2UBGYE47qFQ81zXnrk0YqewJ3kYvgh29JDhF2Jw0Sx0Se1uh3WPi\
        Sr4RhfOJ3UEd200w1zBS0C8TU22BrJKC2D3Wnj0G7egW3NdC340Gn8Kj34QMy633\
        r9mp3t6ujL21lvIt1tPV0i3Z0nod0CvLSu0fwA880Os15W2ZKR0T0V1XR13eEh6s\
        07ozo70VjOHS13oOQD2aj07T3ldSIg1MiXl91jgEhl1qkkcr48Hcbt3UZT7g0UOF\
        GV1ih3KG090Omk11v3od2tC4qc3pVyWU3O67cp3eXg8C2HhVC00yrtyL2TziTx3A\
        fuWH4YbvLl0sAjnL0HQYHx3hyxe83nsCas2TvfcM3mtpjT34CSYQ23y6ef1twU8M\
        2WU8pS4cJehP1QQTBh0INdJp2n1w8t34kxxY2QJPJQ3vFLNj3H09M70Alkhq09g2\
        Fk0bs2Dc2KiPCN0p7ENr4QtkXh2Fg8Su2dIEYY0qhmhY12xLPI0s0VWO3Unc4Y2E\
        i3kJ2bduN32Ziol315CBfx0rEsZz41Gx0M4Mtf7219RwBm3HhJJT1mGXWU3tWVAp\
        2CIgWR3Lob1P4V7B624bbP2F1vTkKv0dEWJ20bTB824Zfkp53sBWeJ3y6OCM10t0\
        aN1aSZv12vVVGl3eTJC80oA4PW128q5C23Zz6I2OpVLZ062fbo2bFVWJ1y4aYO49\
        rCGq1ycHV945ATyQ3DzEd03HVOe83w58N63jaCJ10dmnUd2c67ut0ZbgY22TigI6\
        1UPsIE22FMNV2ZHhJv4SLJLc0RsMrl2Da6NL30aunz2fiPnH1TLRMC0oXgvu2dRc\
        Ol08c9zx2qXm1q2YB2s13zJyBn34tEeN0CCxJi\
    ";

    #[test]
    fn test_load_1() {
        load_blueprint::<Value, Value>(DATA_1).unwrap();
    }

    #[test]
    fn test_load_dump() {
        let value = load_blueprint::<Value, Value>(DATA_2).unwrap();
        let dump = dump_blueprint(value).unwrap();
        assert_eq!(dump, DATA_2);
    }

}


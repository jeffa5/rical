use log::debug;

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

impl OutputFormat {
    pub fn render(&self, formatable: &impl Formatable) -> anyhow::Result<String> {
        match self {
            OutputFormat::Table => Self::render_table(formatable),
            OutputFormat::Json => Self::render_json(formatable),
        }
    }

    fn render_table(formatable: &impl Formatable) -> anyhow::Result<String> {
        let keys = formatable.keys();
        let ts = TableSerializer::new(keys);
        let s = ts.to_string(formatable);
        Ok(s)
    }

    fn render_json(formatable: &impl Formatable) -> anyhow::Result<String> {
        let s = serde_json::to_string(formatable)?;
        Ok(s)
    }
}

pub trait Formatable: serde::Serialize {
    fn keys(&self) -> Vec<String>;
}

impl<T> Formatable for Vec<T>
where
    T: Formatable,
{
    fn keys(&self) -> Vec<String> {
        self.first().map(|v| v.keys()).unwrap_or_default()
    }
}

struct TableSerializer {
    columns: Vec<String>,
    table: comfy_table::Table,
    last_key: Option<String>,
    row: Vec<String>,
    depth: usize,
}

impl TableSerializer {
    fn new(keys: Vec<String>) -> Self {
        let mut table = comfy_table::Table::new();
        table
            .load_preset(comfy_table::presets::NOTHING)
            .set_header(&keys);
        let row = vec![String::new(); keys.len()];
        Self {
            columns: keys,
            last_key: None,
            table,
            row,
            depth: 0,
        }
    }

    fn flush_row(&mut self) {
        self.depth -= 1;
        if self.depth != 0 {
            return;
        }
        self.table.add_row(&self.row);
        for i in 0..self.row.len() {
            self.row[i] = String::new();
        }
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_string(mut self, value: &impl serde::Serialize) -> String {
        value.serialize(&mut self).unwrap();
        self.table.to_string()
    }
}

impl serde::ser::Serializer for &mut TableSerializer {
    type Ok = ();
    type Error = Error;

    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq> {
        Ok(self)
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap> {
        self.depth += 1;
        Ok(self)
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Self::SerializeStruct> {
        self.depth += 1;
        Ok(self)
    }

    fn serialize_bool(self, _: bool) -> Result<Self::Ok> {
        debug!("bool not supported");
        Ok(())
    }

    fn serialize_i8(self, _: i8) -> Result<Self::Ok> {
        debug!("i8 not supported");
        Ok(())
    }

    fn serialize_i16(self, _: i16) -> Result<Self::Ok> {
        debug!("i16 not supported");
        Ok(())
    }

    fn serialize_i32(self, _: i32) -> Result<Self::Ok> {
        debug!("i32 not supported");
        Ok(())
    }

    fn serialize_i64(self, _: i64) -> Result<Self::Ok> {
        debug!("i64 not supported");
        Ok(())
    }

    fn serialize_u8(self, _: u8) -> Result<Self::Ok> {
        debug!("u8 not supported");
        Ok(())
    }

    fn serialize_u16(self, _: u16) -> Result<Self::Ok> {
        debug!("u16 not supported");
        Ok(())
    }

    fn serialize_u32(self, _: u32) -> Result<Self::Ok> {
        debug!("u32 not supported");
        Ok(())
    }

    fn serialize_u64(self, _: u64) -> Result<Self::Ok> {
        debug!("u64 not supported");
        Ok(())
    }

    fn serialize_f32(self, _: f32) -> Result<Self::Ok> {
        debug!("f32 not supported");
        Ok(())
    }

    fn serialize_f64(self, _: f64) -> Result<Self::Ok> {
        debug!("f64 not supported");
        Ok(())
    }

    fn serialize_char(self, _: char) -> Result<Self::Ok> {
        debug!("char not supported");
        Ok(())
    }

    fn serialize_str(self, _s: &str) -> Result<Self::Ok> {
        debug!("str not supported");
        Ok(())
    }

    fn serialize_bytes(self, _: &[u8]) -> Result<Self::Ok> {
        debug!("bytes not supported");
        Ok(())
    }

    fn serialize_none(self) -> Result<Self::Ok> {
        debug!("none not supported");
        Ok(())
    }

    fn serialize_some<T>(self, _: &T) -> Result<Self::Ok>
    where
        T: ?Sized + serde::Serialize,
    {
        debug!("some not supported");
        Ok(())
    }

    fn serialize_unit(self) -> Result<Self::Ok> {
        debug!("unit not supported");
        Ok(())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Self::Ok> {
        debug!("struct not supported");
        Ok(())
    }

    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<Self::Ok> {
        debug!("variant not supported");
        Ok(())
    }

    fn serialize_newtype_struct<T>(self, _: &'static str, _: &T) -> Result<Self::Ok>
    where
        T: ?Sized + serde::Serialize,
    {
        debug!("newtype_struct not supported");
        Ok(())
    }

    fn serialize_newtype_variant<T>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Self::Ok>
    where
        T: ?Sized + serde::Serialize,
    {
        debug!("newtype_variant not supported");
        Ok(())
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple> {
        debug!("tuple not supported");
        Ok(self)
    }

    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        debug!("tuple_struct not supported");
        Ok(self)
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        debug!("tuple_variant not supported");
        Ok(self)
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant> {
        debug!("struct_variant not supported");
        Ok(self)
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
struct Error(String);

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl serde::ser::Error for Error {
    fn custom<T>(msg: T) -> Self
    where
        T: std::fmt::Display,
    {
        Self(msg.to_string())
    }
}

impl serde::ser::SerializeSeq for &mut TableSerializer {
    type Ok = ();
    type Error = Error;

    // Serialize a single element of the sequence.
    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + serde::Serialize,
    {
        value.serialize(&mut **self)
    }

    // Close the sequence.
    fn end(self) -> Result<()> {
        Ok(())
    }
}

impl serde::ser::SerializeMap for &mut TableSerializer {
    type Ok = ();
    type Error = Error;

    fn serialize_key<T>(&mut self, key: &T) -> Result<()>
    where
        T: ?Sized + serde::Serialize,
    {
        let key = serde_plain::to_string(&key).unwrap();
        self.last_key = Some(key.to_owned());
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + serde::Serialize,
    {
        let key = self.last_key.take().unwrap();
        if let Some(pos) = self.columns.iter().position(|i| *i == key) {
            let s = serde_plain::to_string(&value).unwrap();
            self.row[pos] = s;
        } else {
            value.serialize(&mut **self)?;
        }
        Ok(())
    }

    fn end(self) -> Result<()> {
        self.flush_row();
        Ok(())
    }
}

impl serde::ser::SerializeTuple for &mut TableSerializer {
    type Ok = ();
    type Error = Error;

    fn serialize_element<T>(&mut self, _value: &T) -> std::result::Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        todo!()
    }

    fn end(self) -> std::result::Result<Self::Ok, Self::Error> {
        todo!()
    }
}

impl serde::ser::SerializeTupleStruct for &mut TableSerializer {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T>(&mut self, _value: &T) -> std::result::Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        todo!()
    }

    fn end(self) -> std::result::Result<Self::Ok, Self::Error> {
        todo!()
    }
}

impl serde::ser::SerializeTupleVariant for &mut TableSerializer {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T>(&mut self, _value: &T) -> std::result::Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        todo!()
    }

    fn end(self) -> std::result::Result<Self::Ok, Self::Error> {
        todo!()
    }
}

impl serde::ser::SerializeStruct for &mut TableSerializer {
    type Ok = ();
    type Error = Error;

    fn end(self) -> std::result::Result<Self::Ok, Self::Error> {
        self.flush_row();
        Ok(())
    }

    fn serialize_field<T>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> std::result::Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        if let Some(pos) = self.columns.iter().position(|c| *c == key) {
            let s = serde_plain::to_string(&value).unwrap();
            self.row[pos] = s;
        } else {
            value.serialize(&mut **self)?;
        }
        Ok(())
    }
}

impl serde::ser::SerializeStructVariant for &mut TableSerializer {
    type Ok = ();
    type Error = Error;

    fn end(self) -> std::result::Result<Self::Ok, Self::Error> {
        todo!()
    }

    fn serialize_field<T>(
        &mut self,
        _key: &'static str,
        _value: &T,
    ) -> std::result::Result<(), Self::Error>
    where
        T: ?Sized + serde::Serialize,
    {
        todo!()
    }
}

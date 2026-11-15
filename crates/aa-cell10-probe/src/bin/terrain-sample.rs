use anyhow::{Result,ensure};
fn main()->Result<()> {
 let args:Vec<_>=std::env::args().collect();ensure!(args.len()==4,"terrain-sample world x y");
 let world=&args[1];ensure!(world.bytes().all(|b|b.is_ascii_lowercase()||b.is_ascii_digit()||b==b'_'),"invalid world");
 let x:f32=args[2].parse()?;let y:f32=args[3].parse()?;ensure!(x.is_finite()&&y.is_finite(),"invalid position");
 let height=aa_world::TerrainQuery::new(world).height_at(x,y).ok_or_else(||anyhow::anyhow!("no terrain at requested position"))?;
 println!("{}",serde_json::json!({"world":world,"x":x,"y":y,"height":height}));Ok(())
}

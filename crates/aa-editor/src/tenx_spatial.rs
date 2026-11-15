//! Preserve native 10.x octree ownership when an edited item leaves its old node.
use anyhow::{Result,ensure};
use aa_world::placement_write::ObjectTree;
use bevy::math::Vec3;
use std::collections::HashSet;
pub fn origin(bytes:&[u8])->Result<Vec3>{
    ensure!(!bytes.is_empty(),"empty terrain header");let start=if bytes[0]>=24{160}else{32};
    ensure!(bytes.len()>=start+16,"truncated terrain origin");
    let value=|at|f32::from_le_bytes(bytes[at..at+4].try_into().unwrap());
    let out=Vec3::new(value(start+4),value(start+8),0.);ensure!(out.is_finite(),"invalid cell zone origin");Ok(out)
}
fn contains(header:&[u8],p:Vec3)->bool{
    header.len()>=28&&(0..3).all(|i|{
        let lo=f32::from_le_bytes(header[4+i*4..8+i*4].try_into().unwrap());
        let hi=f32::from_le_bytes(header[16+i*4..20+i*4].try_into().unwrap());
        p[i]>=lo-0.01&&p[i]<=hi+0.01
    })
}
/// Returns edited records whose center no longer belongs to their current node.
fn misplaced(tree:&ObjectTree,before:&ObjectTree,origin:Vec3)->Result<Vec<(usize,usize)>>{
    let unchanged:HashSet<_>=before.items.iter().map(|i|(i.kind,i.bytes.as_slice())).collect();
    let mut out=Vec::new();
    for (node,n) in tree.nodes.iter().enumerate(){for &item in &n.items{
        let i=&tree.items[item];
        if i.deleted||unchanged.contains(&(i.kind,i.bytes.as_slice()))||!matches!(i.kind,1|2|9|11|13|14){continue}
        ensure!(i.bytes.len()>=24,"truncated object bounds");
        let f=|k:usize|f32::from_le_bytes(i.bytes[k*4..k*4+4].try_into().unwrap());
        let center=Vec3::new((f(0)+f(3))*0.5,(f(1)+f(4))*0.5,(f(2)+f(5))*0.5)+origin;
        ensure!(center.is_finite()&&contains(&tree.nodes[0].header,center),"edited object lies outside its native zone octree; check destination position and object bounds");
        if !contains(&n.header,center){out.push((node,item));}
    }}Ok(out)
}
pub fn validate(tree:&ObjectTree,before:&ObjectTree,origin:Vec3)->Result<()>{
    ensure!(misplaced(tree,before,origin)?.is_empty(),"edited objects remain in stale octree nodes; reopen and save with the updated 10.x editor");Ok(())
}
/// Rehome only edited records that left their old partition into the existing
/// root, whose bounds cover them. Untouched nodes, records and opaque data stay.
pub fn rehome(tree:&mut ObjectTree,before:&ObjectTree,origin:Vec3)->Result<usize>{
    let moves=misplaced(tree,before,origin)?;
    for &(node,item) in &moves {tree.nodes[node].items.retain(|&i|i!=item);tree.nodes[0].items.push(item);}
    Ok(moves.len())
}
#[cfg(test)]mod tests{
    use super::*;use aa_world::placement_write::{Node,Item};
    fn header(lo:f32,hi:f32)->Vec<u8>{let mut b=2u32.to_le_bytes().to_vec();for x in [lo,lo,0.,hi,hi,1000.]{b.extend(x.to_le_bytes())}b}
    #[test]fn moved_item_uses_zone_origin_and_rehomes_without_changing_payload(){
        let mut data=vec![0;128];for (i,v) in [1f32,1.,1.,3.,3.,3.].iter().enumerate(){data[i*4..i*4+4].copy_from_slice(&v.to_le_bytes())}
        let before=ObjectTree{head:vec![0;8],tail:vec![],nodes:vec![Node{header:header(0.,8192.),mask:1,items:vec![],rest:vec![9],children:vec![1]},Node{header:header(1024.,1088.),mask:0,items:vec![0],rest:vec![],children:vec![]}],items:vec![Item{kind:1,bytes:data,deleted:false}]};
        let mut after=before.clone();after.items[0].bytes[..4].copy_from_slice(&100f32.to_le_bytes());after.items[0].bytes[12..16].copy_from_slice(&102f32.to_le_bytes());
        assert!(validate(&after,&before,Vec3::new(1024.,1024.,0.)).is_err());
        let payload=after.items[0].bytes.clone();assert_eq!(rehome(&mut after,&before,Vec3::new(1024.,1024.,0.)).unwrap(),1);
        assert_eq!(after.nodes[0].items,vec![0]);assert!(after.nodes[1].items.is_empty());assert_eq!(after.nodes[0].rest,vec![9]);assert_eq!(after.items[0].bytes,payload);
        validate(&after,&before,Vec3::new(1024.,1024.,0.)).unwrap();
    }
}
